/* SPDX-License-Identifier: MIT */
/*
 * V3D 7.1 TFU job register values (v3d_tfu71.h), transcribed from Mesa
 * 26.2.2's v3dX(tfu) for V3D_VERSION >= 71
 * (src/gallium/drivers/v3d/v3dx_tfu.c:97-201) with the field positions of
 * src/broadcom/common/v3d_tfu.h:56-78. File:line citations are of the
 * mesa-26.2.2 tag.
 *
 *   Copyright (C) 2021-2022 Raspberry Pi Ltd
 *
 * What moved at 7.1 against 4.2 (v3dx_tfu.c:129-163 against :165-201):
 * IOA is the destination address alone (its 4.2 format and DIMTW bits are
 * gone); the output format, DIMTW, the number of mip levels (NUMMM) and the
 * output stride (padded height in UIF blocks or raster stride in texels,
 * which replaces 4.2's OPAD) are in the new IOC register; ICFG keeps only
 * the input format (bits 23..) and the data type (bits 16..). ICA, IUA and
 * COEF0..3 are 0: Mesa's 7.1 fill never sets them (the zeroed initializer,
 * v3dx_tfu.c:97-105), and its simulator writes them as they are
 * (v3dx_simulator.c:187-188, 193-197).
 */
#include "v3d_tfu71.h"

/* v3d_utile_height (src/broadcom/common/v3d_tiling.c:55-70): the height in
 * pixels of a 64-byte utile; 0 for a cpp it does not know. */
static uint32_t utile_height(uint32_t cpp)
{
	switch (cpp) {
	case 1:
		return 8;
	case 2:
	case 4:
		return 4;
	case 8:
	case 16:
		return 2;
	default:
		return 0;
	}
}

bool v3d_tfu71_supports(uint32_t ttype, bool for_mipmap)
{
	/* v3dx_format_table.c:348-369 (true) and :370-374 (!for_mipmap);
	 * numbers from src/broadcom/cle/v3d_packet.xml:333-369 */
	switch (ttype) {
	case 0:  /* R8,              v3d_packet.xml:333 */
	case 1:  /* R8_SNORM,        :334 */
	case 2:  /* RG8,             :335 */
	case 3:  /* RG8_SNORM,       :336 */
	case 4:  /* RGBA8,           :337 */
	case 5:  /* RGBA8_SNORM,     :338 */
	case 6:  /* RGB565,          :339 */
	case 7:  /* RGBA4,           :340 */
	case 8:  /* RGB5_A1,         :341 */
	case 9:  /* RGB10_A2,        :342 */
	case 10: /* R16,             :343 */
	case 11: /* R16_SNORM,       :344 */
	case 12: /* RG16,            :345 */
	case 13: /* RG16_SNORM,      :346 */
	case 14: /* RGBA16,          :347 */
	case 15: /* RGBA16_SNORM,    :348 */
	case 16: /* R16F,            :349 */
	case 17: /* RG16F,           :350 */
	case 18: /* RGBA16F,         :351 */
	case 19: /* R11F_G11F_B10F,  :352 */
	case 25: /* R4,              :358 */
		return true;
	case 20: /* RGB9_E5,         :353 */
	case 29: /* R32F,            :365 */
	case 30: /* RG32F,           :367 */
	case 31: /* RGBA32F,         :369 */
		return !for_mipmap;
	default:
		return false;
	}
}

/* The format code of a tiling: RASTER's own code, else LINEARTILE's code
 * plus the distance from LINEARTILE (v3dx_tfu.c:166-172 for ICFG,
 * :178-184 for IOC; both codes are 0 and 3, v3d_tfu.h:60-61, 71, 74). */
static uint32_t format_code(enum v3d_tfu71_tiling t, uint32_t raster, uint32_t lineartile)
{
	if (t == V3D_TFU71_TILING_RASTER)
		return raster;
	return lineartile + ((uint32_t)t - (uint32_t)V3D_TFU71_TILING_LINEARTILE);
}

/* The stride field of a side (v3dx_tfu.c:114-127 for IIS, :186-199 for
 * IOC): a UIF side's padded height in UIF blocks (2 utiles high), a raster
 * side's stride in texels, 0 otherwise. False when the value would not
 * describe a `height`-row image of `width` texels. */
static bool side_stride(const struct v3d_tfu71_surface *s, uint32_t cpp, uint32_t width,
			uint32_t height, uint32_t *out)
{
	switch (s->tiling) {
	case V3D_TFU71_TILING_UIF_NO_XOR:
	case V3D_TFU71_TILING_UIF_XOR: {
		const uint32_t ubh = 2u * utile_height(cpp);
		if (s->stride_or_padded_height % ubh != 0 || s->stride_or_padded_height < height)
			return false;
		*out = s->stride_or_padded_height / ubh;
		return true;
	}
	case V3D_TFU71_TILING_RASTER:
		if (s->stride_or_padded_height % cpp != 0 ||
		    (uint64_t)s->stride_or_padded_height < (uint64_t)width * cpp)
			return false;
		*out = s->stride_or_padded_height / cpp;
		return true;
	case V3D_TFU71_TILING_LINEARTILE:
	case V3D_TFU71_TILING_UBLINEAR_1_COLUMN:
	case V3D_TFU71_TILING_UBLINEAR_2_COLUMN:
		*out = 0;
		return true;
	default:
		return false;
	}
}

bool v3d_tfu71_pack(const struct v3d_tfu71_args *a, struct v3d_tfu71_job *j)
{
	if (!a || !j || utile_height(a->cpp) == 0)
		return false;
	/* IOS holds width and height as they are (v3dx_tfu.c:98), at most
	 * V3D_TFU_MAX_DIM each (v3d_tfu.h:27-30) */
	if (a->width == 0 || a->height == 0 || a->width > V3D_TFU71_MAX_DIM ||
	    a->height > V3D_TFU71_MAX_DIM)
		return false;
	const bool for_mipmap = a->num_mm != 0;
	/* v3dx_tfu.c:88-91 */
	if (!v3d_tfu71_supports(a->ttype, for_mipmap))
		return false;
	/* "TFU requires UIF when generating mipmaps", v3dx_tfu.c:64-66 (Mesa
	 * refuses a raster destination there, as here) */
	if (for_mipmap && a->dst.tiling == V3D_TFU71_TILING_RASTER)
		return false;
	/* no more levels than the larger side halves to 1 (ours: Mesa's
	 * last_level never goes there) */
	uint32_t max_side = a->width > a->height ? a->width : a->height, levels = 0;
	while (max_side > 1) {
		max_side >>= 1;
		levels++;
	}
	if (a->num_mm > levels)
		return false;

	uint32_t iis, ostride;
	if (!side_stride(&a->src, a->cpp, a->width, a->height, &iis) ||
	    !side_stride(&a->dst, a->cpp, a->width, a->height, &ostride))
		return false;
	/* IOC's stride field is bits 16..31 (v3d_tfu.h:66) */
	if (ostride > 0xffffu)
		return false;

	struct v3d_tfu71_job out = { 0 }; /* ICA, IUA, COEF0..3: 0 (v3dx_tfu.c:97-105) */
	/* v3dx_tfu.c:98: IOS = height << 16 | width */
	out.ios = a->height << 16 | a->width;
	/* v3dx_tfu.c:106-108: IIA = the source address */
	out.iia = a->src.addr;
	/* v3dx_tfu.c:110-112: IOA = the destination address; nothing else at
	 * 7.1 (the 4.2 format and DIMTW bits, :140-145, are under V3D_VERSION
	 * == 42) */
	out.ioa = a->dst.addr;
	/* v3dx_tfu.c:114-127: IIS */
	out.iis = iis;
	/* v3dx_tfu.c:166-172: ICFG input format at bit 23 (v3d_tfu.h:70) */
	out.icfg = format_code(a->src.tiling, V3D_TFU71_ICFG_FORMAT_RASTER, V3D_TFU71_ICFG_FORMAT_LINEARTILE)
		   << V3D_TFU71_ICFG_IFORMAT_SHIFT;
	/* v3dx_tfu.c:173: ICFG data type at bit 16 (v3d_tfu.h:69) */
	out.icfg |= a->ttype << V3D_TFU71_ICFG_OTYPE_SHIFT;
	/* v3dx_tfu.c:175-176: IOC DIMTW, bit 0 (v3d_tfu.h:57), when levels
	 * after the base are made */
	if (for_mipmap)
		out.ioc |= V3D_TFU71_IOC_DIMTW;
	/* v3dx_tfu.c:178-184: IOC output format at bit 12 (v3d_tfu.h:58) */
	out.ioc |= format_code(a->dst.tiling, V3D_TFU71_IOC_FORMAT_RASTER, V3D_TFU71_IOC_FORMAT_LINEARTILE)
		   << V3D_TFU71_IOC_FORMAT_SHIFT;
	/* v3dx_tfu.c:186-199: IOC stride at bit 16 (v3d_tfu.h:66) */
	out.ioc |= ostride << V3D_TFU71_IOC_STRIDE_SHIFT;
	/* v3dx_tfu.c:201: IOC NUMMM at bit 4 (v3d_tfu.h:67) */
	out.ioc |= a->num_mm << V3D_TFU71_IOC_NUMMM_SHIFT;
	*j = out;
	return true;
}
