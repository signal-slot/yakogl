/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_mip_case.h: a mipmap board test, as data and checks a sample includes
 * (docs/online-shaderc/v3d-backend.md, "Mipmaps on the board"). The same
 * shape as tex_linear_case.h, and tools/v3d/tex_filter_ref_check.c checks it
 * on the host. A regression pin: dEQP's texture.mipmap.2d cases already pass
 * on the Pi 4 (T3, a3f3c39f).
 *
 * The scene, as dEQP's own texture.mipmap.2d.basic cases build theirs: a
 * 64x64 RGBA8 texture with 7 levels, each level ONE colour
 * (es2fTextureMipmapTests.cpp:335, "Doesn't really matter since pixels are
 * unicolored"). The within-level filter (NEAREST or LINEAR) then gives the
 * level's colour whatever the coordinate, so what is judged is the level
 * the TMU picks and how it blends two levels. The within-level filter
 * itself is gl_test 109's (tex_linear_case.h).
 *
 * Draws: a full-viewport quad of size x size pixels, texture coordinates
 * 0..1, so d(texel)/d(pixel) = 64 / size on both axes and the LOD is
 * constant:
 *   size 42: LOD 0.61. dEQP's bounds [0.60, 1.11] allow only level 1 for
 *            *_MIPMAP_NEAREST, so a TMU that rounds down (level 0) fails.
 *   size 14: LOD 2.19. dEQP's bounds [2.19, 2.70] need levels 2 and 3
 *            blended 19..70 % for *_MIPMAP_LINEAR, so a TMU that does not
 *            blend (level 2 or 3 alone) fails.
 * Each draw uses one of the four mipmap min filters (mag NEAREST, CLAMP).
 * Texture U: the 7 levels uploaded one by one, 8 draws (4 filters x 2 sizes).
 * Texture G: level 0 a one-texel checker of TMC_A and TMC_B, the rest made
 * by glGenerateMipmap. Every generated level is then the mean M exactly
 * (a 2x2 box of the checker is 2 A + 2 B, and A + B is even in every
 * channel). 4 draws at size 14 (levels 2 and 3 only), every filter: M.
 *
 * The verdict is dEQP's: transcribed from VK-GL-CTS at 93bca01
 * (Apache-2.0, "Copyright 2014 The Android Open Source Project"):
 * computeFloatingPointError / computeFixedPointError (tcuTexVerifierUtil.cpp
 * :32-47), computeLodBoundsFromDerivates (tcuTexLookupVerifier.cpp
 * :2214-2251), the 2D level choice of isLookupResultValid (:1348-1409),
 * isLinearRangeValid (:250-282), and the precision of
 * es2fTextureMipmapTests.cpp:330-340 (derivateBits 10, lodBits 8, colour
 * threshold of 8 - 1 bits, 8 - 2 for the trilinear filters). With one colour
 * per level, a level lookup is valid when the pixel is within the threshold
 * of that colour, and a two-level one when isLinearRangeValid says so.
 *
 * Header only, no OS calls and no libm (log2 is a series below), so a
 * Zephyr sample and a host test include the same code.
 */
#ifndef TEX_MIP_CASE_H
#define TEX_MIP_CASE_H

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#define TMC_TEX 64
#define TMC_LEVELS 7
#define TMC_DRAWS 12

/* The level colours of texture U, R, G, B, A. The four the draws read
 * (0..3) are 80 apart in R and G; 4..6 only have to differ. Not const: the
 * host check makes two levels alike to see the controls refuse it. */
static uint8_t tmc_level_rgba[TMC_LEVELS][4] = {
	{ 0, 240, 40, 255 },   { 80, 160, 200, 255 }, { 160, 80, 40, 255 }, { 240, 0, 200, 255 },
	{ 128, 128, 90, 255 }, { 60, 200, 120, 255 }, { 200, 60, 160, 255 },
};

/* Texture G's checker colours and their mean. */
static const uint8_t TMC_A[4] = { 250, 10, 100, 255 };
static const uint8_t TMC_B[4] = { 10, 250, 200, 255 };
static const uint8_t TMC_M[4] = { 130, 130, 150, 255 };

enum tmc_min {
	TMC_NEAREST_MIPMAP_NEAREST,
	TMC_LINEAR_MIPMAP_NEAREST,
	TMC_NEAREST_MIPMAP_LINEAR,
	TMC_LINEAR_MIPMAP_LINEAR,
};

struct tmc_draw {
	enum tmc_min min;
	int size;       /* the quad and viewport, pixels */
	bool generated; /* texture G rather than U */
};

static const struct tmc_draw tmc_draws[TMC_DRAWS] = {
	{ TMC_NEAREST_MIPMAP_NEAREST, 42, false }, { TMC_LINEAR_MIPMAP_NEAREST, 42, false },
	{ TMC_NEAREST_MIPMAP_LINEAR, 42, false },  { TMC_LINEAR_MIPMAP_LINEAR, 42, false },
	{ TMC_NEAREST_MIPMAP_NEAREST, 14, false }, { TMC_LINEAR_MIPMAP_NEAREST, 14, false },
	{ TMC_NEAREST_MIPMAP_LINEAR, 14, false },  { TMC_LINEAR_MIPMAP_LINEAR, 14, false },
	{ TMC_NEAREST_MIPMAP_NEAREST, 14, true },  { TMC_LINEAR_MIPMAP_NEAREST, 14, true },
	{ TMC_NEAREST_MIPMAP_LINEAR, 14, true },   { TMC_LINEAR_MIPMAP_LINEAR, 14, true },
};

static inline const char *tmc_min_name(enum tmc_min m)
{
	static const char *const n[4] = { "NEAREST_MIPMAP_NEAREST", "LINEAR_MIPMAP_NEAREST",
					  "NEAREST_MIPMAP_LINEAR", "LINEAR_MIPMAP_LINEAR" };
	return n[m];
}

static inline bool tmc_is_linear_mipmap(enum tmc_min m)
{
	return m == TMC_NEAREST_MIPMAP_LINEAR || m == TMC_LINEAR_MIPMAP_LINEAR;
}

/* ---- the texels ------------------------------------------------------------ */

static uint8_t tmc_texels[TMC_TEX * TMC_TEX * 4];

/* Level k of texture U into tmc_texels (row 0 first), its size returned. */
static inline int tmc_make_level(int k)
{
	int n = TMC_TEX >> k;
	for (int i = 0; i < n * n; i++)
		memcpy(&tmc_texels[i * 4], tmc_level_rgba[k], 4);
	return n;
}

/* Texture G's level 0: TMC_A where x + y is even, TMC_B elsewhere. */
static inline void tmc_make_checker(void)
{
	for (int y = 0; y < TMC_TEX; y++)
		for (int x = 0; x < TMC_TEX; x++)
			memcpy(&tmc_texels[(y * TMC_TEX + x) * 4], ((x + y) & 1) ? TMC_B : TMC_A, 4);
}

/* ---- dEQP's precision ------------------------------------------------------ */

static inline uint32_t tmc_f2u(float f)
{
	uint32_t u;
	memcpy(&u, &f, 4);
	return u;
}

static inline float tmc_u2f(uint32_t u)
{
	float f;
	memcpy(&f, &u, 4);
	return f;
}

/* computeFloatingPointError: the size of the value's mantissa bits past
 * `bits` accurate ones, at the value's exponent */
static inline float tmc_fp_error(float value, int bits)
{
	uint32_t exp = (tmc_f2u(value) >> 23) & 0xff; /* biased */
	uint32_t mask = (1u << (23 - bits)) - 1u;
	return tmc_u2f((exp << 23) | mask) - tmc_u2f(exp << 23);
}

/* computeFixedPointError */
static inline float tmc_fixed_error(int bits)
{
	return tmc_fp_error(1.0f, bits);
}

/* log2 of a positive normal float, without libm: the exponent, plus
 * ln(m) / ln(2) for the mantissa m in [1, 2) by ln(m) = 2 atanh(z),
 * z = (m - 1) / (m + 1) <= 1/3, to z^15 (error below 1e-8). */
static inline float tmc_log2(float v)
{
	uint32_t u = tmc_f2u(v);
	int e = (int)((u >> 23) & 0xff) - 127;
	double m = (double)tmc_u2f((u & 0x7fffffu) | (127u << 23));
	double z = (m - 1.0) / (m + 1.0), z2 = z * z, term = z, sum = 0.0;
	for (int k = 1; k <= 15; k += 2) {
		sum += term / k;
		term *= z2;
	}
	return (float)(e + 2.0 * sum / 0.69314718055994530942);
}

/* computeLodBoundsFromDerivates for du/dx = dv/dy = r, dv/dx = du/dy = 0
 * (an axis-aligned quad), LodPrecision { derivateBits 10, lodBits 8 } */
static inline void tmc_lod_bounds(float r, float *lo, float *hi)
{
	const float p_min = r;                      /* max(mux, mvx) */
	const float p_max = 1.41421356237f * r;     /* sqrt2 * (mux + mvx) */
	const float lod_err = tmc_fixed_error(8);
	*lo = tmc_log2(p_min - tmc_fp_error(p_min, 10)) - lod_err;
	*hi = tmc_log2(p_max + tmc_fp_error(p_max, 10)) + lod_err;
}

/* the colour threshold (colours in 0..1): computeFixedPointThreshold of
 * 8 - 1 bits (8 - 2 for the trilinear filters) */
static inline float tmc_threshold(enum tmc_min m)
{
	return tmc_fixed_error(tmc_is_linear_mipmap(m) ? 6 : 7);
}

static inline int tmc_floor(float f)
{
	int i = (int)f;
	return ((float)i > f) ? i - 1 : i;
}

static inline int tmc_ceil(float f)
{
	int i = (int)f;
	return ((float)i < f) ? i + 1 : i;
}

static inline int tmc_clamp(int v, int lo, int hi)
{
	return v < lo ? lo : (v > hi ? hi : v);
}

static inline float tmc_clampf(float v, float lo, float hi)
{
	return v < lo ? lo : (v > hi ? hi : v);
}

/* the colour of level k of the draw's texture, in 0..1 */
static inline void tmc_level_colour(const struct tmc_draw *d, int k, float out[4])
{
	const uint8_t *c = d->generated ? TMC_M : tmc_level_rgba[k];
	for (int i = 0; i < 4; i++)
		out[i] = (float)c[i] / 255.0f;
}

/* the level-sample check for a one-colour level (isLevelSampleResultValid
 * on it: every texel, and every blend of them, is that colour) */
static inline bool tmc_level_valid(const float c[4], const float px[4], float thr)
{
	for (int i = 0; i < 4; i++) {
		float d = px[i] - c[i];
		if (d > thr || -d > thr)
			return false;
	}
	return true;
}

/* isLinearRangeValid */
static inline bool tmc_linear_range_valid(const float c0[4], const float c1[4], float f0, float f1,
					  const float px[4], float thr)
{
	for (int i = 0; i < 4; i++) {
		float i0 = c0[i] * (1.0f - f0) + c1[i] * f0;
		float i1 = c0[i] * (1.0f - f1) + c1[i] * f1;
		float rmin = px[i] - thr, rmax = px[i] + thr;
		bool smin0 = i0 >= rmin, smin1 = i1 >= rmin, smax0 = i0 > rmax, smax1 = i1 > rmax;
		if (smin0 == smin1 && smin1 == smax0 && smax0 == smax1)
			return false;
	}
	return true;
}

/* The levels a draw may read, as isLookupResultValid chooses them for its
 * LOD bounds (minified: every bound here is above 0): *lo_level..*hi_level
 * for a nearest-mipmap filter, the pairs (k, k + 1) for k in that range for
 * a linear-mipmap one. */
static inline void tmc_levels(const struct tmc_draw *d, float lo, float hi, int *lo_level, int *hi_level)
{
	if (tmc_is_linear_mipmap(d->min)) {
		*lo_level = tmc_clamp(tmc_floor(lo), 0, TMC_LEVELS - 2);
		*hi_level = tmc_clamp(tmc_floor(hi), 0, TMC_LEVELS - 2);
	} else {
		/* "ceil(lod + 0.5) - 1 ... Khronos has made decision to allow
		 * floor(lod + 0.5) as well" */
		*lo_level = tmc_clamp(tmc_ceil(lo + 0.5f) - 1, 0, TMC_LEVELS - 1);
		*hi_level = tmc_clamp(tmc_floor(hi + 0.5f), 0, TMC_LEVELS - 1);
	}
}

/* dEQP's verdict for one read-back pixel (R, G, B, A bytes) of draw d */
static inline bool tmc_pixel_valid(const struct tmc_draw *d, const uint8_t px8[4])
{
	float lo, hi, px[4];
	int l0, l1;
	const float thr = tmc_threshold(d->min);
	tmc_lod_bounds((float)TMC_TEX / (float)d->size, &lo, &hi);
	tmc_levels(d, lo, hi, &l0, &l1);
	for (int i = 0; i < 4; i++)
		px[i] = (float)px8[i] / 255.0f;
	for (int k = l0; k <= l1; k++) {
		float c0[4], c1[4];
		tmc_level_colour(d, k, c0);
		if (!tmc_is_linear_mipmap(d->min)) {
			if (tmc_level_valid(c0, px, thr))
				return true;
			continue;
		}
		tmc_level_colour(d, k + 1, c1);
		if (tmc_linear_range_valid(c0, c1, tmc_clampf(lo - (float)k, 0.0f, 1.0f),
					   tmc_clampf(hi - (float)k, 0.0f, 1.0f), px, thr))
			return true;
	}
	return false;
}

/* ---- the sample's side -------------------------------------------------- */

struct tmc_result {
	unsigned pixels, invalid;
	int first_x, first_y;
	uint8_t first[4];
	uint8_t centre[4];
};

static inline void tmc_result_init(struct tmc_result *r)
{
	memset(r, 0, sizeof(*r));
	r->first_x = r->first_y = -1;
}

/* pixel (x, y) of draw d, GL window coordinates from the lower-left corner */
static inline void tmc_check_pixel(const struct tmc_draw *d, int x, int y, const uint8_t px[4],
				   struct tmc_result *r)
{
	r->pixels++;
	if (x == d->size / 2 && y == d->size / 2)
		memcpy(r->centre, px, 4);
	if (tmc_pixel_valid(d, px))
		return;
	if (r->invalid == 0) {
		r->first_x = x;
		r->first_y = y;
		memcpy(r->first, px, 4);
	}
	r->invalid++;
}

typedef void (*tmc_print_fn)(const char *fmt, ...);

/* the draw's line; true when every pixel of the quad was checked and valid */
static inline bool tmc_verdict(const struct tmc_draw *d, const struct tmc_result *r, tmc_print_fn print)
{
	float lo, hi;
	int l0, l1;
	tmc_lod_bounds((float)TMC_TEX / (float)d->size, &lo, &hi);
	tmc_levels(d, lo, hi, &l0, &l1);
	const unsigned area = (unsigned)(d->size * d->size);
	bool ok = r->pixels == area && r->invalid == 0;
	print("tex_mip %s %s %dx%d: LOD %d.%03d..%d.%03d, levels %d..%d%s; %u of %u pixels invalid; "
	      "centre %u,%u,%u,%u; %s\n",
	      d->generated ? "generated" : "uploaded", tmc_min_name(d->min), d->size, d->size,
	      (int)lo, (int)(lo * 1000.0f) % 1000, (int)hi, (int)(hi * 1000.0f) % 1000, l0,
	      tmc_is_linear_mipmap(d->min) ? l1 + 1 : l1,
	      tmc_is_linear_mipmap(d->min) ? " blended" : "", r->invalid, area, r->centre[0],
	      r->centre[1], r->centre[2], r->centre[3], ok ? "PASS" : "FAIL");
	if (r->invalid)
		print("tex_mip: first invalid pixel (%d, %d) = %u,%u,%u,%u\n", r->first_x, r->first_y,
		      r->first[0], r->first[1], r->first[2], r->first[3]);
	return ok;
}

/* The controls, printed; false when the check could not tell the cases
 * apart:
 * - each draw accepts its own reference colour: for a nearest-mipmap
 *   filter every allowed level's colour, for a linear-mipmap one the blend
 *   at the middle of its first pair's range;
 * - each draw of texture U REJECTS the colour a wrong TMU gives: level 0
 *   alone (no mipmapping), the level below the allowed range (rounding
 *   down), and for a linear-mipmap filter each allowed level alone (no
 *   blending);
 * - each draw of texture G rejects TMC_A, TMC_B and black, and none of its
 *   levels is 0 (the checker, which is not one colour). */
static inline bool tmc_controls(tmc_print_fn print)
{
	unsigned accepts = 0, rejects = 0, wrong = 0;
	const uint8_t black[4] = { 0, 0, 0, 255 };

	for (int i = 0; i < TMC_DRAWS; i++) {
		const struct tmc_draw *d = &tmc_draws[i];
		float lo, hi, c0[4], c1[4];
		int l0, l1;
		uint8_t px[4];

		tmc_lod_bounds((float)TMC_TEX / (float)d->size, &lo, &hi);
		tmc_levels(d, lo, hi, &l0, &l1);
		if (tmc_is_linear_mipmap(d->min)) {
			float f = (tmc_clampf(lo - (float)l0, 0.0f, 1.0f) +
				   tmc_clampf(hi - (float)l0, 0.0f, 1.0f)) * 0.5f;
			tmc_level_colour(d, l0, c0);
			tmc_level_colour(d, l0 + 1, c1);
			for (int c = 0; c < 4; c++)
				px[c] = (uint8_t)((c0[c] * (1.0f - f) + c1[c] * f) * 255.0f + 0.5f);
			accepts++;
			wrong += !tmc_pixel_valid(d, px);
		} else {
			for (int k = l0; k <= l1; k++) {
				memcpy(px, d->generated ? TMC_M : tmc_level_rgba[k], 4);
				accepts++;
				wrong += !tmc_pixel_valid(d, px);
			}
		}
		if (d->generated) {
			wrong += l0 == 0;
			rejects += 3;
			wrong += tmc_pixel_valid(d, TMC_A) + tmc_pixel_valid(d, TMC_B) + tmc_pixel_valid(d, black);
			continue;
		}
		/* level 0 alone, and the level below the range */
		rejects += 1;
		wrong += tmc_pixel_valid(d, tmc_level_rgba[0]);
		if (l0 >= 1) {
			rejects += 1;
			wrong += tmc_pixel_valid(d, tmc_level_rgba[l0 - 1]);
		}
		if (tmc_is_linear_mipmap(d->min) && l0 == l1) {
			/* a range inside one pair: each end level alone is not a blend */
			rejects += 2;
			wrong += tmc_pixel_valid(d, tmc_level_rgba[l0]) + tmc_pixel_valid(d, tmc_level_rgba[l0 + 1]);
		}
	}
	bool ok = wrong == 0;
	print("tex_mip controls: %u reference colours accepted, %u wrong-TMU colours rejected, %u "
	      "exceptions; %s\n",
	      accepts, rejects, wrong, ok ? "the check can tell them apart" : "FAIL");
	return ok;
}

#endif
