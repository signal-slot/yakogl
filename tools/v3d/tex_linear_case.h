/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_linear_case.h: the LINEAR board test, as data and checks a sample
 * includes (docs/online-shaderc/v3d-backend.md, "LINEAR on the board").
 * Stage 1 is v3d-m5's TRI_TEX sample (driver level); stage 2 is a textured
 * v3d_gl_triangle variant (through the GL, after T2). Both include this file,
 * and tools/v3d/tex_filter_ref_check.c checks it on the host.
 *
 * What the sample does:
 *   1. tlc_make_texture() gives the 4x4 RGBA8 texture. Upload it with row 0
 *      first (row 0 is t = 0, as in glTexImage2D).
 *   2. Call tlc_controls() and stop if it returns false. It proves that the
 *      probe can tell LINEAR from NEAREST, a half-texel shift, CLAMP from
 *      REPEAT, and NEAREST from LINEAR on the minified quad.
 *   3. For each of the TLC_DRAWS draws: a quad of size x size pixels whose
 *      texture coordinates run from 0 to reps in s and t, (0, 0) at the quad's
 *      lower-left corner, with the draw's sampler (min, mag, wrap on S and T;
 *      no mipmaps). The fragment shader is gl_FragColor = texture2D(tex, v_uv).
 *   4. Read the quad back. For pixel (x, y), in GL window coordinates relative
 *      to the quad's lower-left corner, call tlc_check_pixel(). Then
 *      tlc_verdict() prints the draw's line and returns pass or fail.
 *
 * Header only, no OS calls, no libm; the caller passes its print function.
 */
#ifndef TEX_LINEAR_CASE_H
#define TEX_LINEAR_CASE_H

#include <stdbool.h>
#include <stdint.h>

#include "tex_filter_ref.h"

#define TLC_TEX_W 4
#define TLC_TEX_H 4

/* dEQP's colour thresholds for an RGBA8 target, in 1/255 steps
 * (es2fTextureFilteringTests.cpp:316-324, tcuTexVerifierUtil.cpp:32-47):
 * colour bits - 2 for LINEAR, - 1 for NEAREST only; computeFixedPointError(n)
 * = (2^(23 - n) - 1) / 2^23. */
#define TLC_DEQP_LINEAR_TOL_LSB (255.0f * (float)((1u << 17) - 1u) / (float)(1u << 23))
#define TLC_DEQP_NEAREST_TOL_LSB (255.0f * (float)((1u << 16) - 1u) / (float)(1u << 23))

struct tlc_draw {
	const char *name;
	int size;           /* the quad is size x size pixels */
	float reps;         /* texture coordinates 0..reps */
	bool min_linear, mag_linear;
	enum tfr_wrap wrap; /* on S and T */
	bool expect_linear; /* the filter the pixels must show */
	float tol_lsb;      /* the largest error allowed, in 1/255 steps */
};

/* (c) and (d) must be exact: a NEAREST texel goes through the f16 return and
 * back to 8 bits unchanged (f16's 11 significant bits keep n/255 within
 * 0.5/255). A non-zero error there is a finding, not noise; dEQP itself would
 * allow TLC_DEQP_NEAREST_TOL_LSB. */
#define TLC_DRAWS 4
static const struct tlc_draw tlc_draws[TLC_DRAWS] = {
	{ "(a) LINEAR, CLAMP_TO_EDGE", 32, 1.0f, true, true, TFR_CLAMP_TO_EDGE, true,
	  TLC_DEQP_LINEAR_TOL_LSB },
	{ "(b) LINEAR, REPEAT", 32, 1.0f, true, true, TFR_REPEAT, true, TLC_DEQP_LINEAR_TOL_LSB },
	{ "(c) NEAREST, CLAMP_TO_EDGE", 32, 1.0f, false, false, TFR_CLAMP_TO_EDGE, false, 0.0f },
	{ "(d) min NEAREST + mag LINEAR, REPEAT, minified (LOD 1)", 16, 8.0f, false, true,
	  TFR_REPEAT, false, 0.0f },
};

static uint8_t tlc_texels[TLC_TEX_W * TLC_TEX_H * 4];

/* R 0/85/170/255 along s, G the same along t, B a 0/255 checker, A 255: every
 * bilinear blend is a different colour, and a swapped s/t or channel shows. */
static inline const uint8_t *tlc_make_texture(void)
{
	static const uint8_t ramp[4] = { 0, 85, 170, 255 };
	for (int j = 0; j < TLC_TEX_H; j++)
		for (int i = 0; i < TLC_TEX_W; i++) {
			uint8_t *p = &tlc_texels[(j * TLC_TEX_W + i) * 4];
			p[0] = ramp[i];
			p[1] = ramp[j];
			p[2] = ((i ^ j) & 1) ? 255 : 0;
			p[3] = 255;
		}
	return tlc_texels;
}

/* The texel coordinate pixel x samples at: (x + 0.5) * texture size * reps / size */
static inline float tlc_coord(const struct tlc_draw *d, int x, int texture_size)
{
	return ((float)x + 0.5f) * (float)texture_size * d->reps / (float)d->size;
}

/* Pixel (x, y) of draw d, sampled LINEAR or NEAREST with wrap w, the
 * coordinates shifted by `shift` texels (0 for the reference; a control uses
 * 0.5). tlc_make_texture() must have run. */
static inline void tlc_sample(const struct tlc_draw *d, bool linear, enum tfr_wrap w, int x, int y,
			      float shift, float out[4])
{
	struct tfr_texture t = { tlc_texels, TLC_TEX_W, TLC_TEX_H };
	float u = tlc_coord(d, x, TLC_TEX_W) + shift, v = tlc_coord(d, y, TLC_TEX_H) + shift;
	if (linear)
		tfr_linear(&t, w, w, u, v, out);
	else
		tfr_nearest(&t, w, w, u, v, out);
}

/* The expected colour of pixel (x, y) of draw d. */
static inline void tlc_expected(const struct tlc_draw *d, int x, int y, float out[4])
{
	tlc_sample(d, d->expect_linear, d->wrap, x, y, 0.0f, out);
}

/* The per-draw measurement: the largest error per channel; a histogram of
 * each pixel's largest error (exactly 0, then < 1, < 2, < 3, < 4, < 8, < 16,
 * >= 16 steps); the pixels over the draw's tolerance and the first of them. */
#define TLC_HIST 8
struct tlc_result {
	float max_lsb[4];
	unsigned hist[TLC_HIST];
	unsigned pixels, over;
	int first_over_x, first_over_y;
};

static inline void tlc_result_init(struct tlc_result *r)
{
	for (int c = 0; c < 4; c++)
		r->max_lsb[c] = 0.0f;
	for (int i = 0; i < TLC_HIST; i++)
		r->hist[i] = 0;
	r->pixels = r->over = 0;
	r->first_over_x = r->first_over_y = -1;
}

/* "0" is at most 1/1000 of a step: the reference is a float */
#define TLC_ZERO_LSB 0.001f

static inline void tlc_check_pixel(const struct tlc_draw *d, int x, int y, const uint8_t px[4],
				   struct tlc_result *r)
{
	static const float upper[TLC_HIST - 2] = { 1.0f, 2.0f, 3.0f, 4.0f, 8.0f, 16.0f };
	float ref[4], per[4];
	tlc_expected(d, x, y, ref);
	float e = tfr_error_lsb(px, ref, per);
	for (int c = 0; c < 4; c++)
		if (per[c] > r->max_lsb[c])
			r->max_lsb[c] = per[c];
	int b = TLC_HIST - 1;
	if (e <= TLC_ZERO_LSB)
		b = 0;
	else
		for (int i = 0; i < TLC_HIST - 2; i++)
			if (e < upper[i]) {
				b = i + 1;
				break;
			}
	r->hist[b]++;
	r->pixels++;
	if (e > d->tol_lsb + TLC_ZERO_LSB) {
		if (r->over == 0) {
			r->first_over_x = x;
			r->first_over_y = y;
		}
		r->over++;
	}
}

/* The caller's print function (printk on Zephyr, printf on the host), so this
 * header needs no OS. */
typedef void (*tlc_print_fn)(const char *fmt, ...);

/* A float in 1/255 steps as whole and hundredths, for a printk without %f */
#define TLC_HUNDREDTHS(f) (int)(f), (int)((f) * 100.0f) % 100

/* The draw's line; true when every pixel was measured and none is over the
 * tolerance. A read-back that measured fewer pixels than the quad has fails:
 * an empty read-back is not a pass. */
static inline bool tlc_verdict(const struct tlc_draw *d, const struct tlc_result *r,
			       tlc_print_fn print)
{
	unsigned want = (unsigned)(d->size * d->size);
	bool ok = r->pixels == want && r->over == 0;
	print("tex_linear %s: %u of %u pixels, %u over %d.%02d; max error R %d.%02d G %d.%02d "
	      "B %d.%02d A %d.%02d; histogram 0:%u <1:%u <2:%u <3:%u <4:%u <8:%u <16:%u "
	      ">=16:%u; %s\n",
	      d->name, r->pixels, want, r->over, TLC_HUNDREDTHS(d->tol_lsb),
	      TLC_HUNDREDTHS(r->max_lsb[0]), TLC_HUNDREDTHS(r->max_lsb[1]),
	      TLC_HUNDREDTHS(r->max_lsb[2]), TLC_HUNDREDTHS(r->max_lsb[3]), r->hist[0], r->hist[1],
	      r->hist[2], r->hist[3], r->hist[4], r->hist[5], r->hist[6], r->hist[7],
	      ok ? "PASS" : "FAIL");
	if (r->over)
		print("tex_linear %s: first pixel over the tolerance at (%d, %d)\n", d->name,
		      r->first_over_x, r->first_over_y);
	return ok;
}

/* Pixels of draw d where sampling a (rounded to 8 bits, as the board would
 * write it) and sampling b differ by more than dEQP's LINEAR threshold. */
static inline int tlc_count_differ(const struct tlc_draw *d, bool lin_a, enum tfr_wrap wa,
				   float shift_a, bool lin_b, enum tfr_wrap wb)
{
	int n = 0;
	for (int y = 0; y < d->size; y++)
		for (int x = 0; x < d->size; x++) {
			float ra[4], rb[4];
			uint8_t px[4];
			tlc_sample(d, lin_a, wa, x, y, shift_a, ra);
			tlc_sample(d, lin_b, wb, x, y, 0.0f, rb);
			for (int c = 0; c < 4; c++)
				px[c] = (uint8_t)(ra[c] * 255.0f + 0.5f);
			if (tfr_error_lsb(px, rb, (float *)0) > TLC_DEQP_LINEAR_TOL_LSB)
				n++;
		}
	return n;
}

/* The counts tlc_controls() measures, for a host test to read. */
struct tlc_control_counts {
	int lin_near, shifted, rep_clamp, min_filter;
	int area, mini;
};

/* The controls, printed; false if the probe could not tell the cases apart.
 * Each must reach a quarter of its quad (REPEAT vs CLAMP: a sixteenth, since
 * only the edges can differ). tlc_make_texture() must have run. */
static inline bool tlc_controls(tlc_print_fn print, struct tlc_control_counts *out)
{
	const struct tlc_draw *a = &tlc_draws[0], *minified = &tlc_draws[3];
	struct tlc_control_counts k;
	k.area = a->size * a->size;
	k.mini = minified->size * minified->size;
	k.lin_near = tlc_count_differ(a, true, TFR_CLAMP_TO_EDGE, 0.0f, false, TFR_CLAMP_TO_EDGE);
	k.shifted = tlc_count_differ(a, true, TFR_CLAMP_TO_EDGE, 0.5f, true, TFR_CLAMP_TO_EDGE);
	k.rep_clamp = tlc_count_differ(a, true, TFR_REPEAT, 0.0f, true, TFR_CLAMP_TO_EDGE);
	k.min_filter = tlc_count_differ(minified, false, TFR_REPEAT, 0.0f, true, TFR_REPEAT);
	bool ok = k.lin_near >= k.area / 4 && k.shifted >= k.area / 4 &&
		  k.rep_clamp >= k.area / 16 && k.min_filter >= k.mini / 4;
	print("tex_linear controls: LINEAR vs NEAREST %d/%d, half-texel shift %d/%d, REPEAT vs "
	      "CLAMP %d/%d, minified NEAREST vs LINEAR %d/%d pixels over the LINEAR tolerance; "
	      "%s\n",
	      k.lin_near, k.area, k.shifted, k.area, k.rep_clamp, k.area, k.min_filter, k.mini,
	      ok ? "the probe can tell them apart" : "FAIL: the probe cannot tell them apart");
	if (out)
		*out = k;
	return ok;
}

#endif
