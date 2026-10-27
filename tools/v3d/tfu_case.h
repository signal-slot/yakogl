/* SPDX-License-Identifier: MIT */
/*
 * tfu_case.h: the TFU probe of v3d_gl_test case 140 (tfuprobe), header only,
 * no OS calls, checked on the host by tools/v3d/tfu_case_check.c.
 *
 * The question (the TFU note and the lead's decision of 2026-09-26 11:20):
 * can the V3D 4.2's texture formatting unit tile a raster image into the
 * layout our CPU tiling writes (v3d_tex_store_level), byte for byte, for the
 * formats GL ES 2.0 textures use here, at odd widths and at source strides
 * and addresses Mesa never uses; and how long it takes against the CPU at
 * 1920x720. No product code submits a TFU job; this is the measurement that
 * decides whether one will.
 *
 * The job's fields are packed by v3d_tex_tfu_pack (driver/src/v3d/v3d_tex.c,
 * as Mesa's v3dX(tfu) does for V3D 4.2, v3dx_tfu.c at b739817b, MIT).
 *
 * For a blit Mesa rewrites the format by texel size (v3dx_tfu.c:63-73:
 * 4 bytes R32F, 2 bytes R16F, 1 byte R8), a byte copy; for a mipmap it keeps
 * the format. The probe runs both types for RGBA8 and RG8.
 */
#ifndef TFU_CASE_H
#define TFU_CASE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "v3d_tex.h"
#include "tex_pixel_offset.h" /* the checker's own texel addresses */

/* The job's registers and its packing are driver/src/v3d/v3d_tex.c's
 * (v3d_tex_tfu_pack), shared with the V3D backend's TFU path. */
#define TFU_TTYPE_R8 V3D_TFU_TTYPE_R8
#define TFU_TTYPE_RG8 V3D_TFU_TTYPE_RG8
#define TFU_TTYPE_RGBA8 V3D_TFU_TTYPE_RGBA8
#define TFU_TTYPE_R16F V3D_TFU_TTYPE_R16F
#define TFU_TTYPE_R32F V3D_TFU_TTYPE_R32F
#define tfu_case_job v3d_tex_tfu_job

static inline bool tfu_pack(const struct v3d_tex_layout *l, uint32_t ttype, uint32_t src_gpu,
			    uint32_t src_stride, uint32_t dst_gpu, struct tfu_case_job *j)
{
	return v3d_tex_tfu_pack(l, ttype, src_gpu, src_stride, dst_gpu, j);
}

/* The probe's formats: the texel size, the job's type, and a name. */
struct tfu_case_fmt {
	const char *name;
	uint32_t cpp, ttype;
};
#define TFU_CASE_FMTS 5
static const struct tfu_case_fmt tfu_case_fmts[TFU_CASE_FMTS] = {
	{ "RGBA8 as R32F", 4, TFU_TTYPE_R32F },   /* Mesa's blit type for 4 bytes */
	{ "RGBA8 as RGBA8", 4, TFU_TTYPE_RGBA8 }, /* Mesa's mipmap type */
	{ "RG8 as R16F", 2, TFU_TTYPE_R16F },     /* Mesa's blit type for 2 bytes */
	{ "RG8 as RG8", 2, TFU_TTYPE_RG8 },
	{ "R8", 1, TFU_TTYPE_R8 },
};

/* Odd widths that reach every tiling for each texel size (1, 2 and 4
 * bytes): LINEARTILE, UBLINEAR 1 and 2 columns, UIF, and UIF XOR with UIF
 * block padding (OPAD 5): 95x419 for 1 byte, 95x211 for 2 and 4. The host
 * check asserts the coverage. */
#define TFU_CASE_SIZES 8
static const uint32_t tfu_case_sizes[TFU_CASE_SIZES][2] = {
	{ 3, 5 }, { 7, 9 }, { 13, 11 }, { 29, 19 }, { 95, 37 }, { 61, 515 }, { 95, 211 }, { 95, 419 },
};

/* The source variants: the stride (rounded up to 64 bytes, as every Mesa
 * raster source is, or tight = width * cpp) and the source address's offset
 * from a page boundary (0 as Mesa's, 64, 4). The first is Mesa's shape: the
 * verdict. The others are findings. */
#define TFU_CASE_VARIANTS 6
struct tfu_case_variant {
	const char *name;
	bool aligned_stride;
	uint32_t src_offset;
};
static const struct tfu_case_variant tfu_case_variants[TFU_CASE_VARIANTS] = {
	{ "s64+0", true, 0 }, { "s64+64", true, 64 }, { "s64+4", true, 4 },
	{ "tight+0", false, 0 }, { "tight+64", false, 64 }, { "tight+4", false, 4 },
};

static inline uint32_t tfu_case_stride(uint32_t width, uint32_t cpp, bool aligned)
{
	uint32_t s = width * cpp;
	return aligned ? (s + 63u) & ~63u : s;
}

/* The fill of both images before a tiling, and a source texel byte: byte
 * k of texel (x, y) under `seed`, never the fill, different in most
 * neighbours so a texel in the wrong place is found. The board case gives
 * every job its own seed: the jobs reuse one source buffer, and a TFU that
 * read a stale copy of the previous job's source would otherwise match. */
#define TFU_CASE_FILL 0xeeu
static inline uint8_t tfu_case_byte(uint32_t x, uint32_t y, uint32_t k, uint32_t seed)
{
	uint32_t v = (x * 0x9e3779b1u) ^ (y * 0x85ebca77u) ^ (k * 0xc2b2ae3du) ^ (seed * 0x27d4eb2fu);
	v ^= v >> 15;
	uint8_t b = (uint8_t)(v ^ (v >> 8));
	return b == TFU_CASE_FILL ? 0x11u : b;
}

/* Write the raster source: height rows of width texels, `stride` apart; the
 * bytes between rows are left alone. */
static inline void tfu_case_fill_src(uint8_t *src, uint32_t width, uint32_t height, uint32_t cpp,
				     uint32_t stride, uint32_t seed)
{
	for (uint32_t y = 0; y < height; y++)
		for (uint32_t x = 0; x < width; x++)
			for (uint32_t k = 0; k < cpp; k++)
				src[(size_t)y * stride + (size_t)x * cpp + k] = tfu_case_byte(x, y, k, seed);
}

/* A comparison of a job's image with the CPU's, over level 0's slice. */
struct tfu_case_cmp {
	uint32_t texels;          /* texels compared: width * height */
	uint32_t texel_wrong;     /* texels with a byte unlike the CPU's */
	uint32_t texel_unwritten; /* of those, texels whose every byte is still the fill */
	uint32_t pad_bytes;       /* bytes of the slice that are no texel's */
	uint32_t pad_written;     /* of those, bytes no longer the fill */
	uint32_t first_x, first_y; /* the first wrong texel (row order); UINT32_MAX: none */
};

/* Compare `got` (the job's image, filled with TFU_CASE_FILL before it) with
 * `ref` (v3d_tex_store_level's image of the same source and layout, filled
 * the same way before it) at every texel of level 0, through
 * v3d_tex_pixel_offset; and count the slice's other bytes the job wrote
 * (padding: the TMU never reads it, so a write there is a finding, not a
 * failure). A texel byte is never the fill, so the reference's fill bytes
 * are exactly the padding. */
static inline void tfu_case_compare(const uint8_t *ref, const uint8_t *got, const struct v3d_tex_layout *l,
				    struct tfu_case_cmp *c)
{
	const struct v3d_tex_slice *s = &l->slices[0];
	const uint32_t cpp = l->cpp;
	*c = (struct tfu_case_cmp){ 0 };
	c->first_x = c->first_y = UINT32_MAX;
	for (uint32_t y = 0; y < l->height; y++)
		for (uint32_t x = 0; x < l->width; x++) {
			const uint32_t o = s->offset + v3d_tex_pixel_offset(s->tiling, cpp, s->padded_height, x, y);
			bool wrong = false, unwritten = true;
			for (uint32_t k = 0; k < cpp; k++) {
				wrong |= got[o + k] != ref[o + k];
				unwritten &= got[o + k] == TFU_CASE_FILL;
			}
			c->texels++;
			if (wrong) {
				if (c->texel_wrong == 0) {
					c->first_x = x;
					c->first_y = y;
				}
				c->texel_wrong++;
				c->texel_unwritten += unwritten;
			}
		}
	for (uint32_t i = 0; i < s->size; i++)
		if (ref[s->offset + i] == TFU_CASE_FILL) {
			c->pad_bytes++;
			c->pad_written += got[s->offset + i] != TFU_CASE_FILL;
		}
}

static inline const char *tfu_case_tiling_name(enum v3d_tex_tiling t)
{
	switch (t) {
	case V3D_TEX_TILING_LINEARTILE: return "LT";
	case V3D_TEX_TILING_UBLINEAR_1_COLUMN: return "UBL1";
	case V3D_TEX_TILING_UBLINEAR_2_COLUMN: return "UBL2";
	case V3D_TEX_TILING_UIF_NO_XOR: return "UIF";
	case V3D_TEX_TILING_UIF_XOR: return "UIFXOR";
	}
	return "?";
}

#endif
