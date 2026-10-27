/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_cubemip_case.h: a cube-map mipmap board test, as data and checks a
 * sample includes (docs/online-shaderc/v3d-backend.md, "Cube mipmaps on the
 * board"). The same shape as tex_mip_case.h, and
 * tools/v3d/tex_filter_ref_check.c checks it on the host.
 *
 * Why: dEQP-GLES2 texture.mipmap.cube.{basic,bias,projected} fail on the
 * Pi 4 (batch 23, 8f2c90a3), nearest_nearest included. For basic and bias,
 * dEQP's reference differs from an ES3 context's only in seamlessCubeMap
 * (es2fTextureMipmapTests.cpp:658; lodBits is 6 either way, :669), and a
 * NEAREST sample never reads across a face. This case therefore samples
 * face INTERIORS only, where seamless filtering cannot matter, and asks the
 * two things left: does each face's level L sit where the TMU reads it, and
 * does the TMU pick the level dEQP allows?
 *
 * The scene, as dEQP's cube mipmap cases build theirs: a 64x64 RGBA8 cube
 * with 7 levels, every (face, level) one colour, dEQP's formula
 * (es2fTextureMipmapTests.cpp TextureCubeMipmapCase::init). Per face, 7
 * draws of a quad of size x size pixels whose direction is
 * major + h * (x * sx + y * ty) (tex_cube_case.h's face table, x and y the
 * quad's -1..1), so the face coordinates run over [0.5 - h/2, 0.5 + h/2],
 * and the texels per pixel are r = 64 h / size on both axes, constant:
 *   NEAREST_MIPMAP_NEAREST at sizes 38, 19, 9, 5 (h 0.5) and 4 (h 0.84):
 *     LOD about -0.25, 0.75, 1.83, 2.68, 3.75, where dEQP allows exactly
 *     one level each: 0, 1, 2, 3, 4;
 *   LINEAR_MIPMAP_LINEAR at sizes 14 and 7 (h 0.5): LOD about 1.19, 2.19,
 *     levels 1..2 and 2..3 blended 18..71 %, with every LINEAR footprint
 *     inside the face (s, t in [0.25, 0.75]).
 * Mag NEAREST, CLAMP_TO_EDGE.
 *
 * The verdict is dEQP's: computeCubeLodBoundsFromDerivates
 * (tcuTexLookupVerifier.cpp:2264-2320 at 93bca01, Apache-2.0,
 * "Copyright 2014 The Android Open Source Project") for this geometry
 * (the major axis constant 1, its derivative 0: du/dx = dv/dy = r), the
 * cube case's precision (derivateBits 10, lodBits 6 for a non-projected
 * look-up; colour threshold of 8 - 2 bits, es2fTextureMipmapTests.cpp
 * :660-669), and the level choice and isLinearRangeValid of
 * tex_mip_case.h. Each draw also names the (face, level) colour its centre
 * pixel is nearest to, so a wrong placement names itself.
 *
 * Header only, no OS calls and no libm.
 */
#ifndef TEX_CUBEMIP_CASE_H
#define TEX_CUBEMIP_CASE_H

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "tex_cube_case.h" /* tcc_faces: sx, ty per face */
#include "tex_mip_case.h"  /* dEQP's error terms, log2, level checks */

#define TCM_SIZE 64
#define TCM_LEVELS 7
#define TCM_PER_FACE 7
#define TCM_DRAWS (6 * TCM_PER_FACE)

/* dEQP's per-(face, level) colour. dEQP numbers faces -X, +X, -Y, +Y, -Z,
 * +Z; this header uses GL's order +X, -X, +Y, -Y, +Z, -Z. */
static inline void tcm_colour(int face, int level, uint8_t out[4])
{
	static const int deqp_face[6] = { 1, 0, 3, 2, 5, 4 };
	uint32_t step = 0xffu / (TCM_LEVELS - 1);
	uint32_t inc = step * (uint32_t)level;
	uint32_t dec = 0xffu - inc;
	uint32_t rgb = 0;
	switch (deqp_face[face]) {
	case 0: rgb = (inc << 16) | (dec << 8) | 255u; break;
	case 1: rgb = (255u << 16) | (inc << 8) | dec; break;
	case 2: rgb = (dec << 16) | (255u << 8) | inc; break;
	case 3: rgb = (dec << 16) | (inc << 8) | 255u; break;
	case 4: rgb = (255u << 16) | (dec << 8) | inc; break;
	default: rgb = (inc << 16) | (255u << 8) | dec; break;
	}
	out[0] = (uint8_t)(rgb >> 16);
	out[1] = (uint8_t)(rgb >> 8);
	out[2] = (uint8_t)rgb;
	out[3] = 255;
}

static uint8_t tcm_texels[TCM_SIZE * TCM_SIZE * 4];

/* Level `level` of face `face` into tcm_texels (one colour), its size. */
static inline int tcm_make(int face, int level)
{
	uint8_t c[4];
	int n = TCM_SIZE >> level;
	tcm_colour(face, level, c);
	for (int i = 0; i < n * n; i++)
		memcpy(&tcm_texels[i * 4], c, 4);
	return n;
}

struct tcm_draw {
	bool linear; /* LINEAR_MIPMAP_LINEAR, else NEAREST_MIPMAP_NEAREST */
	int size;    /* the quad and viewport, pixels */
	float h;     /* half the face-coordinate span */
};

/* the draws of one face */
static const struct tcm_draw tcm_face_draws[TCM_PER_FACE] = {
	{ false, 38, 0.5f }, { false, 19, 0.5f }, { false, 9, 0.5f }, { false, 5, 0.5f },
	{ false, 4, 0.84f }, { true, 14, 0.5f },  { true, 7, 0.5f },
};

/* texels per pixel on both axes */
static inline float tcm_r(const struct tcm_draw *d)
{
	return (float)TCM_SIZE * d->h / (float)d->size;
}

/* computeCubeLodBoundsFromDerivates for this geometry, then
 * computeLodBoundsFromDerivates with LodPrecision { 10, 6 } */
static inline void tcm_lod_bounds(const struct tcm_draw *d, float *lo, float *hi)
{
	const float r = tcm_r(d);
	const float p_max = 1.41421356237f * r;
	const float lod_err = tmc_fixed_error(6);
	*lo = tmc_log2(r - tmc_fp_error(r, 10)) - lod_err;
	*hi = tmc_log2(p_max + tmc_fp_error(p_max, 10)) + lod_err;
}

/* the colour threshold: 8 - 2 bits (a non-projected cube case) */
static inline float tcm_threshold(void)
{
	return tmc_fixed_error(6);
}

/* the levels the draw may read (tex_mip_case.h's rule, 7 levels); a draw
 * that can be magnified (lo <= 0, mag NEAREST) reads level 0 then */
static inline void tcm_levels(const struct tcm_draw *d, float lo, float hi, int *l0, int *l1)
{
	if (d->linear) {
		*l0 = tmc_clamp(tmc_floor(lo), 0, TCM_LEVELS - 2);
		*l1 = tmc_clamp(tmc_floor(hi), 0, TCM_LEVELS - 2);
	} else {
		*l0 = tmc_clamp(tmc_ceil(lo + 0.5f) - 1, 0, TCM_LEVELS - 1);
		*l1 = tmc_clamp(tmc_floor(hi + 0.5f), 0, TCM_LEVELS - 1);
	}
}

static inline void tcm_colourf(int face, int level, float out[4])
{
	uint8_t c[4];
	tcm_colour(face, level, c);
	for (int i = 0; i < 4; i++)
		out[i] = (float)c[i] / 255.0f;
}

/* dEQP's verdict for one read-back pixel of draw d on face `face` */
static inline bool tcm_pixel_valid(int face, const struct tcm_draw *d, const uint8_t px8[4])
{
	float lo, hi, px[4];
	int l0, l1;
	const float thr = tcm_threshold();
	tcm_lod_bounds(d, &lo, &hi);
	tcm_levels(d, lo, hi, &l0, &l1);
	for (int i = 0; i < 4; i++)
		px[i] = (float)px8[i] / 255.0f;
	if (lo <= 0.0f) { /* can be magnified: mag NEAREST reads level 0 */
		float c[4];
		tcm_colourf(face, 0, c);
		if (tmc_level_valid(c, px, thr))
			return true;
	}
	for (int k = l0; k <= l1; k++) {
		float c0[4], c1[4];
		tcm_colourf(face, k, c0);
		if (!d->linear) {
			if (tmc_level_valid(c0, px, thr))
				return true;
			continue;
		}
		tcm_colourf(face, k + 1, c1);
		if (tmc_linear_range_valid(c0, c1, tmc_clampf(lo - (float)k, 0.0f, 1.0f),
					   tmc_clampf(hi - (float)k, 0.0f, 1.0f), px, thr))
			return true;
	}
	return false;
}

/* the (face, level) whose colour is nearest to `px` (largest channel
 * difference), for the diagnosis line */
static inline void tcm_nearest_colour(const uint8_t px[4], int *face, int *level, int *dist)
{
	*face = *level = -1;
	*dist = 1 << 30;
	for (int f = 0; f < 6; f++)
		for (int l = 0; l < TCM_LEVELS; l++) {
			uint8_t c[4];
			int worst = 0;
			tcm_colour(f, l, c);
			for (int i = 0; i < 3; i++) {
				int dd = (int)px[i] - (int)c[i];
				if (dd < 0)
					dd = -dd;
				if (dd > worst)
					worst = dd;
			}
			if (worst < *dist) {
				*dist = worst;
				*face = f;
				*level = l;
			}
		}
}

struct tcm_result {
	unsigned pixels, invalid;
	int first_x, first_y;
	uint8_t first[4], centre[4];
};

static inline void tcm_result_init(struct tcm_result *r)
{
	memset(r, 0, sizeof(*r));
	r->first_x = r->first_y = -1;
}

/* pixel (x, y) of draw d on `face`, GL window coordinates */
static inline void tcm_check_pixel(int face, const struct tcm_draw *d, int x, int y, const uint8_t px[4],
				   struct tcm_result *r)
{
	r->pixels++;
	if (x == d->size / 2 && y == d->size / 2)
		memcpy(r->centre, px, 4);
	if (tcm_pixel_valid(face, d, px))
		return;
	if (r->invalid == 0) {
		r->first_x = x;
		r->first_y = y;
		memcpy(r->first, px, 4);
	}
	r->invalid++;
}

typedef void (*tcm_print_fn)(const char *fmt, ...);

static inline bool tcm_verdict(int face, const struct tcm_draw *d, const struct tcm_result *r,
			       tcm_print_fn print)
{
	static const char *const names[6] = { "+X", "-X", "+Y", "-Y", "+Z", "-Z" };
	float lo, hi;
	int l0, l1, cf, cl, cd;
	tcm_lod_bounds(d, &lo, &hi);
	tcm_levels(d, lo, hi, &l0, &l1);
	tcm_nearest_colour(r->centre, &cf, &cl, &cd);
	const unsigned area = (unsigned)(d->size * d->size);
	bool ok = r->pixels == area && r->invalid == 0;
	int lo_c = (int)(lo * 100.0f), hi_c = (int)(hi * 100.0f);
	print("tex_cubemip %s %s %dx%d: LOD %s%d.%02d..%d.%02d, levels %d..%d%s; %u of %u pixels invalid; "
	      "centre %u,%u,%u reads %s level %d (off %d); %s\n",
	      names[face], d->linear ? "LINEAR_MIPMAP_LINEAR" : "NEAREST_MIPMAP_NEAREST", d->size, d->size,
	      lo_c < 0 ? "-" : "", (lo_c < 0 ? -lo_c : lo_c) / 100, (lo_c < 0 ? -lo_c : lo_c) % 100,
	      hi_c / 100, hi_c % 100, l0, d->linear ? l1 + 1 : l1, d->linear ? " blended" : "", r->invalid,
	      area, r->centre[0], r->centre[1], r->centre[2], cf >= 0 ? names[cf] : "?", cl, cd,
	      ok ? "PASS" : "FAIL");
	if (r->invalid)
		print("tex_cubemip: first invalid pixel (%d, %d) = %u,%u,%u,%u\n", r->first_x, r->first_y,
		      r->first[0], r->first[1], r->first[2], r->first[3]);
	return ok;
}

/* The controls, printed; false when the check could not tell the cases
 * apart: every draw accepts its own reference colour (the allowed level,
 * or the middle of its blend) and REJECTS what a wrong TMU gives: the
 * levels next to the allowed one on the same face (a wrong LOD or a level
 * found at the wrong place), and for a blend each end level alone. Every
 * draw's face coordinates stay inside the face (h < 1), a NEAREST draw
 * allows exactly one level, and a LINEAR draw's footprints stay inside the
 * face on both of its levels (no texel from another face). */
static inline bool tcm_controls(tcm_print_fn print)
{
	unsigned accepts = 0, rejects = 0, wrong = 0;
	for (int f = 0; f < 6; f++)
		for (int i = 0; i < TCM_PER_FACE; i++) {
			const struct tcm_draw *d = &tcm_face_draws[i];
			float lo, hi;
			int l0, l1;
			uint8_t px[4];
			tcm_lod_bounds(d, &lo, &hi);
			tcm_levels(d, lo, hi, &l0, &l1);
			wrong += d->h >= 1.0f;
			if (d->linear) {
				/* the smallest face coordinate, at the coarser level:
				 * s * n - 0.5 must be >= 0 (texel 0's centre or later) */
				float smin = 0.5f - d->h / 2.0f;
				int n1 = TCM_SIZE >> (l0 + 1);
				wrong += l0 != l1;
				wrong += smin * (float)n1 < 0.5f;
				float c0[4], c1[4];
				float fm = (tmc_clampf(lo - (float)l0, 0.0f, 1.0f) + tmc_clampf(hi - (float)l0, 0.0f, 1.0f)) / 2.0f;
				tcm_colourf(f, l0, c0);
				tcm_colourf(f, l0 + 1, c1);
				for (int c = 0; c < 4; c++)
					px[c] = (uint8_t)((c0[c] * (1.0f - fm) + c1[c] * fm) * 255.0f + 0.5f);
				accepts++;
				wrong += !tcm_pixel_valid(f, d, px);
				rejects += 2;
				tcm_colour(f, l0, px);
				wrong += tcm_pixel_valid(f, d, px);
				tcm_colour(f, l0 + 1, px);
				wrong += tcm_pixel_valid(f, d, px);
				continue;
			}
			wrong += l0 != l1; /* exactly one level allowed */
			tcm_colour(f, l0, px);
			accepts++;
			wrong += !tcm_pixel_valid(f, d, px);
			for (int k = l0 - 1; k <= l0 + 1; k += 2) {
				if (k < 0 || k >= TCM_LEVELS)
					continue;
				rejects++;
				tcm_colour(f, k, px);
				wrong += tcm_pixel_valid(f, d, px);
			}
		}
	bool ok = wrong == 0;
	print("tex_cubemip controls: %u reference colours accepted, %u wrong-level colours rejected, %u "
	      "exceptions; %s\n",
	      accepts, rejects, wrong, ok ? "the check can tell them apart" : "FAIL");
	return ok;
}

#endif
