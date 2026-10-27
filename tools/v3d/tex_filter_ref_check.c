/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * tex_filter_ref_check: checks tools/v3d/tex_filter_ref.h (dEQP's filters),
 * tools/v3d/tex_linear_case.h (the LINEAR board test the samples run) and
 * tools/v3d/tex_cube_case.h (the cube-map one;
 * docs/online-shaderc/v3d-backend.md, "LINEAR on the board") before any
 * board time:
 *
 *  1. Pixels of draw (a)'s scene computed by hand (the 4x4 ramp on a 32x32
 *     quad, pixel x at u = (x + 0.5) / 8 texels): LINEAR with CLAMP_TO_EDGE,
 *     REPEAT and MIRRORED_REPEAT, and NEAREST.
 *  2. The plan's premise: every LINEAR weight of every draw is k/16, so the
 *     board result cannot depend on the hardware's weight precision if it
 *     has 4 fraction bits or more.
 *  3. tlc_controls(), the check the sample runs before its draws, with its
 *     counts; and it refuses a flat texture, where nothing can be told apart.
 *  4. tlc_verdict() on read-backs made on the host: an ideal read-back (the
 *     reference rounded to 8 bits) passes every draw; a NEAREST read-back of a
 *     LINEAR draw, a half-texel shift, a LINEAR read-back of the minified draw,
 *     a short read-back, and one pixel off by one step in an exact draw each
 *     fail; an error of 3 steps in a LINEAR draw passes (dEQP allows 3.98).
 *  5. Cube maps (tools/v3d/tex_cube_case.h): dEQP's face selection and
 *     projection on hand-computed directions, its tie rules; the case's
 *     controls (its face table sends every pixel of every face where dEQP
 *     says, the filters and neighbouring faces can be told apart) and their
 *     refusal of faces that look alike; two pixels by hand; the verdict on
 *     host-made read-backs (ideal passes all 12 draws; the wrong filter, the
 *     wrong face and a short read-back fail; a "seamless" edge is measured,
 *     not judged).
 *  6. Mipmaps (tools/v3d/tex_mip_case.h): the libm-free log2 and dEQP's
 *     error terms on known values; dEQP's LOD bounds of both draw sizes
 *     against numbers derived by hand from computeLodBoundsFromDerivates;
 *     the levels each filter may read; the case's controls, and their
 *     refusal when two levels look alike; the verdict on host-made
 *     read-backs (an ideal TMU passes all 12 draws; no mipmapping, rounding
 *     down at size 42, no blending, a blend past a level, levels never generated, a short
 *     read-back and an error past dEQP's threshold each fail; an error inside it passes).
 *  7. Cube mipmaps (tools/v3d/tex_cubemip_case.h): dEQP's cube LOD bounds of
 *     the 7 draws against numbers derived by hand; the one level each
 *     NEAREST draw allows and the pair each LINEAR draw blends; the case's
 *     controls; the verdict on host-made read-backs (an exact TMU passes all
 *     42 draws; the level next to the right one, the right level found on
 *     another face, no mipmapping, no blending and a short read-back each
 *     fail; 3 steps off passes and 5 steps off fails, dEQP allowing 3.98).
 *  9. Narrow L8, A8 and LA8 (tools/v3d/tex_narrow_case.h): the draws' pixel
 *     count; two cube pixels by hand; what each format samples as (ES 2.0
 *     table 3.12); the case's controls; the verdict on
 *     host-made read-backs (an exact TMU passes all 60 draws; each of the
 *     seven wrong reads fails every draw it applies to; RGBA8's swizzle on
 *     an R8 or RG8 texture fails every draw; a short read-back fails; one
 *     step off passes, two steps fail).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Itools/v3d \
 *      tools/v3d/tex_filter_ref_check.c -o build/tex_filter_ref_check
 *
 * Exit status 0 only when every check passed AND the number run is the number
 * planned.
 */
#include <stdarg.h>
#include <stdio.h>

#include "tex_linear_case.h"
#include "tex_cube_case.h"
#include "tex_mip_case.h"
#include "tex_cubemip_case.h"
#include "tex_cubethin_case.h"
#include "tex_narrow_case.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("tex_filter_ref_check: FAIL %s\n", what);
	}
}

static void print_out(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
}

static void print_none(const char *fmt, ...)
{
	(void)fmt;
}

/* ---- 1. hand-computed pixels ------------------------------------------- */

static void hand_case(const char *name, bool linear, enum tfr_wrap w, int x, int y, int c,
		      float want_255)
{
	float out[4];
	tlc_sample(&tlc_draws[0], linear, w, x, y, 0.0f, out);
	float got = out[c] * 255.0f, d = got - want_255;
	char what[160];
	snprintf(what, sizeof(what), "%s pixel (%d, %d) channel %d: %.4f, by hand %.4f", name, x, y,
		 c, (double)got, (double)want_255);
	check(d < 1e-3f && d > -1e-3f, what);
}

static void part_hand(void)
{
	/* R depends on x only: u - 0.5 = (2x - 7) / 16 */
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 0, 16, 0, 0.0f);
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 4, 16, 0, 85.0f / 16.0f);
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 15, 16, 0, 85.0f + 85.0f * 7.0f / 16.0f);
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 31, 16, 0, 255.0f);
	/* x = 0: texels 3 and 0, a = frac(-7/16) = 9/16 */
	hand_case("LINEAR REPEAT", true, TFR_REPEAT, 0, 16, 0, 255.0f * 7.0f / 16.0f);
	/* x = 31: texels 3 and 0, a = 7/16 */
	hand_case("LINEAR REPEAT", true, TFR_REPEAT, 31, 16, 0, 255.0f * 9.0f / 16.0f);
	/* x = 0: both texels map to 0 */
	hand_case("LINEAR MIRRORED", true, TFR_MIRRORED_REPEAT, 0, 16, 0, 0.0f);
	/* G depends on y only, the same ramp */
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 16, 4, 1, 85.0f / 16.0f);
	/* B, the checker, at (4, 4): a = b = 1/16, the two 255 corners weigh
	 * a(1 - b) + (1 - a)b = 30/256 */
	hand_case("LINEAR CLAMP", true, TFR_CLAMP_TO_EDGE, 4, 4, 2, 255.0f * 30.0f / 256.0f);
	hand_case("NEAREST", false, TFR_CLAMP_TO_EDGE, 0, 16, 0, 0.0f);
	hand_case("NEAREST", false, TFR_CLAMP_TO_EDGE, 15, 16, 0, 85.0f);
	hand_case("NEAREST", false, TFR_CLAMP_TO_EDGE, 31, 16, 0, 255.0f);
}

/* ---- 2. every LINEAR weight is k/16 ------------------------------------ */

static void part_weights(void)
{
	int not_k16 = 0, seen = 0;
	for (int i = 0; i < TLC_DRAWS; i++)
		for (int x = 0; x < tlc_draws[i].size; x++) {
			float a = tfr_frac(tlc_coord(&tlc_draws[i], x, TLC_TEX_W) - 0.5f) * 16.0f;
			seen++;
			if (a != (float)(int)a)
				not_k16++;
		}
	char what[160];
	snprintf(what, sizeof(what), "every LINEAR weight of the %d draws is k/16 (%d of %d are not)",
		 TLC_DRAWS, not_k16, seen);
	check(seen == 32 * 3 + 16 && not_k16 == 0, what);
}

/* ---- 3. the controls ---------------------------------------------------- */

static void part_controls(void)
{
	struct tlc_control_counts k;
	bool ok = tlc_controls(print_out, &k);
	char what[200];
	snprintf(what, sizeof(what),
		 "tlc_controls: %d %d %d %d (the measured counts when written: 960 1008 448 256)",
		 k.lin_near, k.shifted, k.rep_clamp, k.min_filter);
	/* pinned: a change to the scene must change this line on purpose */
	check(ok && k.lin_near == 960 && k.shifted == 1008 && k.rep_clamp == 448 &&
		      k.min_filter == 256,
	      what);

	/* and they can say no: on a flat texture no probe can tell anything
	 * apart, so the controls must refuse the run */
	for (int i = 0; i < TLC_TEX_W * TLC_TEX_H * 4; i++)
		tlc_texels[i] = 128;
	bool flat_ok = tlc_controls(print_none, &k);
	tlc_make_texture();
	check(!flat_ok && k.lin_near == 0, "tlc_controls refuses a flat texture");
}

/* ---- 4. the verdict on host-made read-backs ----------------------------- */

enum readback { RB_IDEAL, RB_NEAREST, RB_LINEAR, RB_SHIFTED, RB_SHORT, RB_ONE_OFF, RB_PLUS3 };

/* Draw d's verdict on a read-back made as `kind` says. */
static bool verdict_of(const struct tlc_draw *d, enum readback kind)
{
	struct tlc_result r;
	tlc_result_init(&r);
	for (int y = 0; y < d->size; y++) {
		if (kind == RB_SHORT && y == d->size - 1)
			break;
		for (int x = 0; x < d->size; x++) {
			bool linear = d->expect_linear;
			if (kind == RB_NEAREST)
				linear = false;
			if (kind == RB_LINEAR)
				linear = true;
			float ref[4];
			tlc_sample(d, linear, d->wrap, x, y, kind == RB_SHIFTED ? 0.5f : 0.0f, ref);
			uint8_t px[4];
			for (int c = 0; c < 4; c++)
				px[c] = (uint8_t)(ref[c] * 255.0f + 0.5f);
			if (kind == RB_ONE_OFF && x == 5 && y == 7)
				px[0] = (uint8_t)(px[0] ^ 1);
			if (kind == RB_PLUS3 && x == 5 && y == 7)
				px[1] = (uint8_t)(px[1] > 128 ? px[1] - 3 : px[1] + 3);
			tlc_check_pixel(d, x, y, px, &r);
		}
	}
	return tlc_verdict(d, &r, print_none);
}

static void verdict_case(int draw, enum readback kind, bool want, const char *what_kind)
{
	char what[200];
	bool got = verdict_of(&tlc_draws[draw], kind);
	snprintf(what, sizeof(what), "verdict %s on %s: %s, want %s", tlc_draws[draw].name,
		 what_kind, got ? "PASS" : "FAIL", want ? "PASS" : "FAIL");
	check(got == want, what);
}

static void part_verdicts(void)
{
	for (int i = 0; i < TLC_DRAWS; i++)
		verdict_case(i, RB_IDEAL, true, "the ideal read-back");
	verdict_case(0, RB_NEAREST, false, "a NEAREST read-back");
	verdict_case(0, RB_SHIFTED, false, "a half-texel shift");
	verdict_case(3, RB_LINEAR, false, "a LINEAR read-back (the wrong filter chosen)");
	verdict_case(0, RB_SHORT, false, "a read-back one row short");
	verdict_case(2, RB_ONE_OFF, false, "one pixel off by one step (exact draw)");
	verdict_case(0, RB_PLUS3, true, "one pixel off by 3 steps (dEQP allows 3.98)");
	/* one verdict line, as the board prints it */
	struct tlc_result r;
	tlc_result_init(&r);
	const struct tlc_draw *d = &tlc_draws[0];
	for (int y = 0; y < d->size; y++)
		for (int x = 0; x < d->size; x++) {
			float ref[4];
			uint8_t px[4];
			tlc_expected(d, x, y, ref);
			for (int c = 0; c < 4; c++)
				px[c] = (uint8_t)(ref[c] * 255.0f + 0.5f);
			tlc_check_pixel(d, x, y, px, &r);
		}
	check(tlc_verdict(d, &r, print_out), "the printed verdict of the ideal draw (a) passes");
}

/* ---- 5. cube maps: dEQP's face selection and tex_cube_case.h -------------- */

static bool near_f(float a, float b)
{
	return tfr_abs(a - b) < 1e-6f;
}

/* Draw `face`/`linear`'s verdict on a host-made read-back: the reference
 * rounded to 8 bits, then changed as `kind` says. */
enum cube_rb { CRB_IDEAL, CRB_NEAREST, CRB_WRONG_FACE, CRB_SEAMLESS_EDGE, CRB_SHORT };

static bool cube_verdict_of(int face, bool linear, enum cube_rb kind)
{
	struct tcc_result r;
	tcc_result_init(&r);
	for (int y = 0; y < TCC_SIZE; y++)
		for (int x = 0; x < TCC_SIZE; x++) {
			if (kind == CRB_SHORT && y == TCC_SIZE - 1 && x == TCC_SIZE - 1)
				continue;
			float ref[4];
			int f = kind == CRB_WRONG_FACE ? (face + 1) % 6 : face;
			tcc_expected(f, kind == CRB_NEAREST ? false : linear, x, y, ref);
			uint8_t px[4];
			for (int c = 0; c < 4; c++)
				px[c] = (uint8_t)(ref[c] * 255.0f + 0.5f);
			/* a seamless TMU blends the next face in at the edge: far
			 * from the clamp reference, and not judged */
			if (kind == CRB_SEAMLESS_EDGE && tcc_edge(x, y))
				px[2] = (uint8_t)(px[2] + 20);
			tcc_check_pixel(face, linear, x, y, px, &r);
		}
	return tcc_verdict(face, linear, &r, print_none);
}

static void part_cube(void)
{
	char what[200];
	float s, t;

	/* dEQP's selection and projection, by hand */
	enum tfr_cube_face f = tfr_select_cube_face(1.0f, 0.5f, -0.25f);
	tfr_project_to_face(f, 1.0f, 0.5f, -0.25f, &s, &t);
	check(f == TFR_PX && near_f(s, 0.625f) && near_f(t, 0.25f),
	      "cube: (1, 0.5, -0.25) is +X at s 0.625, t 0.25 (sc = -z, tc = -y)");
	f = tfr_select_cube_face(0.5f, -1.0f, 0.25f);
	tfr_project_to_face(f, 0.5f, -1.0f, 0.25f, &s, &t);
	check(f == TFR_NY && near_f(s, 0.75f) && near_f(t, 0.375f),
	      "cube: (0.5, -1, 0.25) is -Y at s 0.75, t 0.375 (sc = +x, tc = -z)");
	check(tfr_select_cube_face(1.0f, 1.0f, 0.5f) == TFR_PX &&
		      tfr_select_cube_face(0.5f, -1.0f, -1.0f) == TFR_NY,
	      "cube: dEQP's tie rules (|x| = |y| > |z| picks X; |y| = |z| > |x| picks Y)");

	/* the case's controls, and their refusal when the faces look alike */
	tcc_make_faces();
	check(tcc_controls(print_out), "cube: tcc_controls pass on the case's faces");
	for (int k = 0; k < 6; k++)
		for (int i = 0; i < TCC_TEX * TCC_TEX; i++)
			tcc_texels[k][i * 4 + 2] = 99;
	check(!tcc_controls(print_none), "cube: tcc_controls refuse faces that look alike");
	tcc_make_faces();

	/* two pixels by hand: +Z NEAREST at (16, 16) is texel (2, 2): R = G = 170,
	 * B = 16 + 40 * 4 = 176; -X LINEAR at (4, 16): R = 85/16 (u - 0.5 = 1/16),
	 * B = 56 on every texel of the face */
	float out[4];
	tcc_expected(TFR_PZ, false, 16, 16, out);
	snprintf(what, sizeof(what), "cube: +Z NEAREST (16, 16) = %.2f %.2f %.2f, by hand 170 170 176",
		 (double)(out[0] * 255), (double)(out[1] * 255), (double)(out[2] * 255));
	check(tfr_abs(out[0] * 255.0f - 170) < 1e-3f && tfr_abs(out[1] * 255.0f - 170) < 1e-3f &&
		      tfr_abs(out[2] * 255.0f - 176) < 1e-3f,
	      what);
	tcc_expected(TFR_NX, true, 4, 16, out);
	snprintf(what, sizeof(what), "cube: -X LINEAR (4, 16) R = %.4f, by hand 5.3125; B %.2f, by hand 56",
		 (double)(out[0] * 255), (double)(out[2] * 255));
	check(tfr_abs(out[0] * 255.0f - 85.0f / 16.0f) < 1e-3f && tfr_abs(out[2] * 255.0f - 56) < 1e-3f,
	      what);

	/* the verdict */
	int ideal = 0;
	for (int i = 0; i < TCC_DRAWS; i++)
		ideal += cube_verdict_of(i / 2, i & 1, CRB_IDEAL);
	snprintf(what, sizeof(what), "cube: the ideal read-back passes %d of %d draws", ideal, TCC_DRAWS);
	check(ideal == TCC_DRAWS, what);
	check(!cube_verdict_of(TFR_PY, true, CRB_NEAREST),
	      "cube: a NEAREST read-back of a LINEAR draw fails");
	check(!cube_verdict_of(TFR_NZ, false, CRB_WRONG_FACE),
	      "cube: the neighbouring face's texels fail");
	check(cube_verdict_of(TFR_PX, true, CRB_SEAMLESS_EDGE),
	      "cube: LINEAR edge pixels far from the clamp reference are measured, not judged");
	check(!cube_verdict_of(TFR_NX, false, CRB_SHORT), "cube: a read-back one pixel short fails");
}

/* ---- 6. mipmaps ----------------------------------------------------------- */

enum mip_readback {
	MRB_IDEAL,        /* an exact TMU: LOD log2(64 / size), level round(LOD) or the blend */
	MRB_LEVEL0,       /* no mipmapping: level 0 */
	MRB_ROUND_DOWN,   /* a nearest-mipmap filter that takes floor(LOD) */
	MRB_NO_BLEND,     /* a linear-mipmap filter that takes floor(LOD) alone */
	MRB_BLACK,        /* the generated levels never made: the incomplete texture */
	MRB_SHORT,        /* one pixel fewer than the quad */
	MRB_OFF_3,        /* the ideal, one channel of one pixel 3 steps off */
	MRB_OVERSHOOT,    /* a blend of levels 0 and 1 carried 10 % past level 1 */
};

/* the colour an ideal TMU (or one of the wrong ones) gives for draw d */
static void mip_colour(const struct tmc_draw *d, enum mip_readback how, uint8_t out[4])
{
	float lod = tmc_log2((float)TMC_TEX / (float)d->size), c0[4], c1[4];
	int k = tmc_floor(lod);
	float f = lod - (float)k;

	if (how == MRB_BLACK) {
		const uint8_t black[4] = { 0, 0, 0, 255 };
		memcpy(out, black, 4);
		return;
	}
	if (how == MRB_OVERSHOOT) {
		tmc_level_colour(d, 0, c0);
		tmc_level_colour(d, 1, c1);
		for (int c = 0; c < 4; c++) {
			float v = (c0[c] * -0.1f + c1[c] * 1.1f) * 255.0f + 0.5f;
			out[c] = (uint8_t)(v < 0.0f ? 0.0f : (v > 255.0f ? 255.0f : v));
		}
		return;
	}
	if (how == MRB_LEVEL0) {
		memcpy(out, d->generated ? TMC_A : tmc_level_rgba[0], 4);
		return;
	}
	if (how == MRB_ROUND_DOWN || how == MRB_NO_BLEND) {
		tmc_level_colour(d, k, c0);
		for (int c = 0; c < 4; c++)
			out[c] = (uint8_t)(c0[c] * 255.0f + 0.5f);
		return;
	}
	if (!tmc_is_linear_mipmap(d->min)) {
		tmc_level_colour(d, tmc_floor(lod + 0.5f), c0);
		for (int c = 0; c < 4; c++)
			out[c] = (uint8_t)(c0[c] * 255.0f + 0.5f);
		return;
	}
	tmc_level_colour(d, k, c0);
	tmc_level_colour(d, k + 1, c1);
	for (int c = 0; c < 4; c++)
		out[c] = (uint8_t)((c0[c] * (1.0f - f) + c1[c] * f) * 255.0f + 0.5f);
}

static bool mip_verdict_of(int draw, enum mip_readback how)
{
	const struct tmc_draw *d = &tmc_draws[draw];
	struct tmc_result r;
	uint8_t px[4];
	mip_colour(d, how, px);
	tmc_result_init(&r);
	for (int y = 0; y < d->size; y++)
		for (int x = 0; x < d->size; x++) {
			uint8_t p[4];
			if (how == MRB_SHORT && x == d->size - 1 && y == d->size - 1)
				continue;
			memcpy(p, px, 4);
			if (how == MRB_OFF_3 && x == 3 && y == 5)
				p[1] = (uint8_t)(p[1] + 3);
			tmc_check_pixel(d, x, y, p, &r);
		}
	return tmc_verdict(d, &r, print_none);
}

static bool near_d(double a, double b, double eps)
{
	return a - b < eps && b - a < eps;
}

static void part_mip(void)
{
	char what[220];
	float lo, hi;

	check(near_d(tmc_log2(1.5f), 0.5849625007, 1e-6) && near_d(tmc_log2(3.0f), 1.5849625007, 1e-6) &&
		      near_d(tmc_log2(1024.0f), 10.0, 1e-6) && near_d(tmc_log2(0.75f), -0.4150374993, 1e-6),
	      "mip: log2 of 1.5, 3, 1024 and 0.75 within 1e-6");
	check(tmc_fixed_error(8) == 32767.0f / 8388608.0f &&
		      tmc_fp_error(4.5714285f, 10) == 4.0f * 8191.0f / 8388608.0f &&
		      near_d(tmc_threshold(TMC_NEAREST_MIPMAP_NEAREST) * 255.0f, 1.9921571, 1e-5) &&
		      near_d(tmc_threshold(TMC_LINEAR_MIPMAP_LINEAR) * 255.0f, 3.9843446, 1e-5),
	      "mip: dEQP's error terms (lodBits 8, derivateBits 10 at 4.57, thresholds 1.99 and 3.98)");

	/* by hand from computeLodBoundsFromDerivates: r = 64 / size;
	 * lo = log2(r - err10(r)) - err8, hi = log2(sqrt2 r + err10(sqrt2 r)) + err8 */
	tmc_lod_bounds(64.0f / 42.0f, &lo, &hi);
	snprintf(what, sizeof(what), "mip: size 42 LOD bounds [%.6f, %.6f], by hand [0.602852, 1.112896]",
		 (double)lo, (double)hi);
	check(near_d(lo, 0.602852, 2e-5) && near_d(hi, 1.112896, 2e-5), what);
	tmc_lod_bounds(64.0f / 14.0f, &lo, &hi);
	snprintf(what, sizeof(what), "mip: size 14 LOD bounds [%.6f, %.6f], by hand [2.187506, 2.697423]",
		 (double)lo, (double)hi);
	check(near_d(lo, 2.187506, 2e-5) && near_d(hi, 2.697423, 2e-5), what);

	/* the levels: size 42 nearest-mipmap only level 1; linear-mipmap the
	 * pairs from level 0 and 1; size 14 nearest 2..3, linear the pair 2-3 */
	int want[4][2] = { { 1, 1 }, { 0, 1 }, { 2, 3 }, { 2, 2 } }, got_ok = 1;
	for (int i = 0; i < 4; i++) {
		const struct tmc_draw *d = &tmc_draws[(i / 2) * 4 + (i % 2) * 2];
		int l0, l1;
		tmc_lod_bounds(64.0f / (float)d->size, &lo, &hi);
		tmc_levels(d, lo, hi, &l0, &l1);
		got_ok &= l0 == want[i][0] && l1 == want[i][1];
	}
	check(got_ok, "mip: the levels dEQP allows (42: nearest 1, linear pairs 0-1; 14: nearest 2-3, linear pair 2-3)");

	check(tmc_controls(print_out), "mip: tmc_controls pass on the case's levels");
	uint8_t keep[4];
	memcpy(keep, tmc_level_rgba[3], 4);
	memcpy(tmc_level_rgba[3], tmc_level_rgba[2], 4);
	check(!tmc_controls(print_none), "mip: tmc_controls refuse levels 2 and 3 alike (no blend to see)");
	memcpy(tmc_level_rgba[3], keep, 4);

	int ideal = 0;
	for (int i = 0; i < TMC_DRAWS; i++)
		ideal += mip_verdict_of(i, MRB_IDEAL);
	snprintf(what, sizeof(what), "mip: an exact TMU passes %d of %d draws", ideal, TMC_DRAWS);
	check(ideal == TMC_DRAWS, what);
	check(!mip_verdict_of(0, MRB_LEVEL0), "mip: no mipmapping (level 0) fails NEAREST_MIPMAP_NEAREST 42");
	/* at size 14 dEQP allows levels 2 and 3, so rounding down is legal
	 * there; at 42 only level 1 is, and floor(0.61) is level 0 */
	check(!mip_verdict_of(1, MRB_ROUND_DOWN), "mip: rounding the LOD down fails LINEAR_MIPMAP_NEAREST 42");
	check(!mip_verdict_of(7, MRB_NO_BLEND), "mip: no blending fails LINEAR_MIPMAP_LINEAR 14");
	check(!mip_verdict_of(10, MRB_BLACK), "mip: levels never generated (black) fail the generated draw");
	check(!mip_verdict_of(1, MRB_SHORT), "mip: a read-back one pixel short fails");
	check(!mip_verdict_of(3, MRB_OVERSHOOT),
	      "mip: a blend carried past level 1 fails LINEAR_MIPMAP_LINEAR 42 (the range stops at f = 1)");
	check(!mip_verdict_of(0, MRB_OFF_3) && mip_verdict_of(3, MRB_OFF_3),
	      "mip: 3 steps off fails a nearest-mipmap draw (1.99 allowed), passes a trilinear one (3.98)");
}

/* ---- 7. cube mipmaps ------------------------------------------------------- */

enum cm_readback {
	CRM_IDEAL,      /* an exact TMU: LOD log2(r), level round(LOD) or the blend */
	CRM_NEXT_LEVEL, /* the level after the right one */
	CRM_OTHER_FACE, /* the right level, found on the next face */
	CRM_LEVEL0,     /* no mipmapping */
	CRM_NO_BLEND,   /* a blend draw that reads floor(LOD) alone */
	CRM_SHORT,      /* one pixel fewer than the quad */
	CRM_OFF3,       /* the ideal, one channel of one pixel 3 steps off */
	CRM_OFF5,       /* the same, 5 steps off */
};

static void cm_colour(int face, const struct tcm_draw *d, enum cm_readback how, uint8_t out[4])
{
	float lod = tmc_log2(tcm_r(d));
	int k = tmc_floor(lod), n = tmc_clamp(tmc_floor(lod + 0.5f), 0, TCM_LEVELS - 1);
	float f = lod - (float)k, c0[4], c1[4];
	if (lod < 0.0f)
		n = 0;
	switch (how) {
	case CRM_NEXT_LEVEL: tcm_colour(face, n + 1, out); return;
	case CRM_OTHER_FACE: tcm_colour((face + 1) % 6, n, out); return;
	case CRM_LEVEL0: tcm_colour(face, 0, out); return;
	case CRM_NO_BLEND: tcm_colour(face, k, out); return;
	default: break;
	}
	if (!d->linear) {
		tcm_colour(face, n, out);
		return;
	}
	tcm_colourf(face, k, c0);
	tcm_colourf(face, k + 1, c1);
	for (int c = 0; c < 4; c++)
		out[c] = (uint8_t)((c0[c] * (1.0f - f) + c1[c] * f) * 255.0f + 0.5f);
}

static bool cm_verdict_of(int face, int draw, enum cm_readback how)
{
	const struct tcm_draw *d = &tcm_face_draws[draw];
	struct tcm_result r;
	uint8_t px[4];
	cm_colour(face, d, how, px);
	tcm_result_init(&r);
	for (int y = 0; y < d->size; y++)
		for (int x = 0; x < d->size; x++) {
			uint8_t p[4];
			if (how == CRM_SHORT && x == d->size - 1 && y == d->size - 1)
				continue;
			memcpy(p, px, 4);
			if ((how == CRM_OFF3 || how == CRM_OFF5) && x == 1 && y == 1)
				p[2] = (uint8_t)(p[2] >= 128 ? p[2] - (how == CRM_OFF3 ? 3 : 5) : p[2] + (how == CRM_OFF3 ? 3 : 5));
			tcm_check_pixel(face, d, x, y, p, &r);
		}
	return tcm_verdict(face, d, &r, print_none);
}

static void part_cubemip(void)
{
	/* by hand from computeCubeLodBoundsFromDerivates (r = 64 h / size, the
	 * major axis constant), derivateBits 10, lodBits 6 */
	static const double want[TCM_PER_FACE][2] = {
		{ -0.264389, 0.268880 }, { 0.735611, 1.268880 }, { 1.813658, 2.346820 }, { 2.661566, 3.194941 },
		{ 3.731998, 4.265271 },  { 1.175787, 1.709141 }, { 2.175787, 2.709141 },
	};
	static const int want_levels[TCM_PER_FACE][2] = { { 0, 0 }, { 1, 1 }, { 2, 2 }, { 3, 3 }, { 4, 4 }, { 1, 1 }, { 2, 2 } };
	int bounds_ok = 1, levels_ok = 1;
	for (int i = 0; i < TCM_PER_FACE; i++) {
		float lo, hi;
		int l0, l1;
		tcm_lod_bounds(&tcm_face_draws[i], &lo, &hi);
		bounds_ok &= near_d(lo, want[i][0], 2e-5) && near_d(hi, want[i][1], 2e-5);
		tcm_levels(&tcm_face_draws[i], lo, hi, &l0, &l1);
		levels_ok &= l0 == want_levels[i][0] && l1 == want_levels[i][1];
	}
	check(bounds_ok, "cubemip: the 7 draws' LOD bounds match the numbers derived by hand");
	check(levels_ok, "cubemip: NEAREST allows exactly levels 0, 1, 2, 3, 4; LINEAR blends 1-2 and 2-3");
	check(tcm_controls(print_out), "cubemip: tcm_controls pass");

	int ideal = 0;
	for (int f = 0; f < 6; f++)
		for (int i = 0; i < TCM_PER_FACE; i++)
			ideal += cm_verdict_of(f, i, CRM_IDEAL);
	char what[120];
	snprintf(what, sizeof(what), "cubemip: an exact TMU passes %d of %d draws", ideal, TCM_DRAWS);
	check(ideal == TCM_DRAWS, what);
	check(!cm_verdict_of(2, 2, CRM_NEXT_LEVEL), "cubemip: the next level fails the +Y level-2 draw");
	check(!cm_verdict_of(4, 3, CRM_OTHER_FACE), "cubemip: level 3 of the next face fails the +Z level-3 draw");
	check(!cm_verdict_of(1, 2, CRM_LEVEL0), "cubemip: no mipmapping fails the -X level-2 draw");
	check(!cm_verdict_of(3, 6, CRM_NO_BLEND), "cubemip: no blending fails the -Y 2-3 LINEAR draw");
	check(!cm_verdict_of(0, 1, CRM_SHORT), "cubemip: a read-back one pixel short fails");
	check(cm_verdict_of(5, 1, CRM_OFF3) && !cm_verdict_of(5, 1, CRM_OFF5),
	      "cubemip: 3 steps off passes, 5 steps off fails (dEQP allows 3.98)");
}

/* ---- 8. thin cube cells ---------------------------------------------------- */

enum ct_readback {
	CTR_IDEAL,       /* an exact TMU: level round(log2(64 / thin side)) */
	CTR_FINER_SPLIT, /* the pixels of cut quads one level finer (dEQP's board result) */
	CTR_FINER_ALL,   /* every pixel one level finer */
};

static bool ct_verdict_of(int face, int cell, enum ct_readback how, struct tct_result *r)
{
	const struct tct_cell *c = &tct_cells[cell];
	const struct tcm_draw d = tct_draw(c);
	const int n = tmc_floor(tmc_log2(tcm_r(&d)) + 0.5f);
	tct_result_init(r);
	for (int y = c->y0; y < c->y0 + c->h; y++)
		for (int x = c->x0; x < c->x0 + c->w; x++) {
			const bool finer = how == CTR_FINER_ALL || (how == CTR_FINER_SPLIT && tct_split(c, x, y));
			uint8_t px[4];
			tcm_colour(face, finer ? n - 1 : n, px);
			tct_check_pixel(face, c, x, y, px, r);
		}
	return tct_verdict(0, face, c, r, print_none);
}

static void part_cubethin(void)
{
	/* by hand (dEQP's computeLodBoundsFromDerivates, du/dx = 64 / w,
	 * dv/dy = 64 / h, derivateBits 10, lodBits 6): dEQP's cells 14 and 15 */
	static const double want[TCT_CELLS][2] = {
		{ 1.896164, 2.429220 }, { 3.175787, 3.709141 }, { 1.896164, 2.429220 }, { 3.175787, 3.709141 },
	};
	int bounds_ok = 1, split_ok = 1;
	for (int i = 0; i < TCT_CELLS; i++) {
		const struct tcm_draw d = tct_draw(&tct_cells[i]);
		float lo, hi;
		tcm_lod_bounds(&d, &lo, &hi);
		bounds_ok &= near_d(lo, want[i][0], 2e-5) && near_d(hi, want[i][1], 2e-5);
	}
	/* the cut quads are where dEQP's are: the 17-wide cell's last column
	 * (x 56), the 7-wide cell's first (x 57), and the turned cells' rows */
	split_ok &= tct_split(&tct_cells[0], 56, 10) && !tct_split(&tct_cells[0], 40, 10) &&
		    !tct_split(&tct_cells[0], 55, 63);
	split_ok &= tct_split(&tct_cells[1], 57, 0) && !tct_split(&tct_cells[1], 63, 0);
	split_ok &= tct_split(&tct_cells[2], 3, 56) && !tct_split(&tct_cells[2], 63, 40);
	split_ok &= tct_split(&tct_cells[3], 0, 57) && !tct_split(&tct_cells[3], 0, 63);
	check(bounds_ok, "cubethin: the 4 cells' LOD bounds match dEQP's cells 14 and 15, derived by hand");
	check(split_ok, "cubethin: the cut quads are the 17-wide cell's last line and the 7-wide cell's first");
	check(tct_controls(print_out), "cubethin: tct_controls pass");

	struct tct_result r;
	int ideal = 0;
	for (int f = 0; f < 6; f++)
		for (int i = 0; i < TCT_CELLS; i++)
			ideal += ct_verdict_of(f, i, CTR_IDEAL, &r);
	check(ideal == 6 * TCT_CELLS, "cubethin: an exact TMU passes every cell of every face");
	bool v = ct_verdict_of(4, 0, CTR_FINER_SPLIT, &r);
	check(!v && r.bad_split == 64 && r.bad_whole == 0,
	      "cubethin: dEQP's board result (the cut column one level finer) fails, 64 split-quad pixels only");
	v = ct_verdict_of(0, 1, CTR_FINER_ALL, &r);
	check(!v && r.bad_split == 64 && r.bad_whole == 7 * 64 - 64,
	      "cubethin: a whole cell one level finer fails in both classes");
}

/* ---- 9. narrow L8, A8 and LA8 (tex_narrow_case.h) ------------------------ */

enum nr_readback { NR_IDEAL, NR_WRONG /* + w */, NR_RGBA8_SWIZZLE = NR_WRONG + TNC_WRONG, NR_OFF1, NR_OFF2 };

/* the pixel a host-made TMU returns for pixel (x, y) of draw d; false when
 * the mode does not apply to that pixel (the ideal value is returned) */
static bool nr_pixel(enum tnc_format f, const struct tnc_draw *d, int mode, int x, int y, uint8_t out[4])
{
	tnc_expected(f, d, x, y, out);
	if (mode >= NR_WRONG && mode < NR_WRONG + TNC_WRONG)
		return tnc_wrong_read(f, d, mode - NR_WRONG, x, y, out);
	if (mode == NR_RGBA8_SWIZZLE) {
		/* R8 / RG8 read through RGBA8's XYZW: (b0, 0, 0, 1) or (b0, b1, 0, 1) */
		int layer, level, tx, ty;
		tnc_texel(d, x, y, &layer, &level, &tx, &ty);
		out[0] = tnc_byte(d->kind, layer, level, tx, ty, 0);
		out[1] = f == TNC_LA8 ? tnc_byte(d->kind, layer, level, tx, ty, 1) : 0;
		out[2] = 0;
		out[3] = 255;
		return true;
	}
	if (mode == NR_OFF1 || mode == NR_OFF2) {
		int step = mode == NR_OFF1 ? 1 : 2;
		/* the channel that moves: A for A8, R otherwise, away from its end */
		int c = f == TNC_A8 ? 3 : 0;
		out[c] = (uint8_t)(out[c] >= 128 ? out[c] - step : out[c] + step);
		return true;
	}
	return mode == NR_IDEAL;
}

/* the verdict of draw i of format f on a host read-back; `applied` counts
 * the pixels the mode changed */
static bool nr_verdict(enum tnc_format f, int i, int mode, bool short_by_one, unsigned *applied)
{
	struct tnc_draw d = tnc_draw_of(i);
	struct tnc_result r;
	tnc_result_init(&r);
	*applied = 0;
	for (int y = 0; y < d.size_h; y++)
		for (int x = 0; x < d.size_w; x++) {
			if (short_by_one && y == d.size_h - 1 && x == d.size_w - 1)
				continue;
			uint8_t px[4];
			*applied += nr_pixel(f, &d, mode, x, y, px);
			tnc_check_pixel(f, &d, x, y, px, &r);
		}
	return tnc_verdict(f, i, &r, print_none);
}

static void part_narrow(void)
{
	unsigned area = 0;
	for (int i = 0; i < TNC_DRAWS; i++) {
		struct tnc_draw d = tnc_draw_of(i);
		area += (unsigned)(d.size_w * d.size_h);
	}
	check(area * TNC_FORMATS == TNC_ALL_PIXELS, "narrow: the 20 draws of a format cover 7148 pixels");

	/* by hand: +X level 0 pixel (0, 0): x, y = -15/16, direction
	 * (1, 15/16, 15/16), sc = -z, tc = -y, so s = t = 1/32: texel (0, 0);
	 * +Z level 2 pixel (3, 0): x = 0.75, y = -0.75, direction
	 * (0.75, 0.75, 1), s = 0.875, t = 0.125: texel (3, 0) of the 4x4 level */
	int layer, level, tx, ty, ok = 1;
	struct tnc_draw d = tnc_draw_of(TNC_CHAIN_LEVELS + 1 + 0);
	tnc_texel(&d, 0, 0, &layer, &level, &tx, &ty);
	ok &= d.kind == TNC_KIND_CUBE && d.face == 0 && layer == 0 && level == 0 && tx == 0 && ty == 0;
	d = tnc_draw_of(TNC_CHAIN_LEVELS + 1 + 4 * 2 + 1);
	tnc_texel(&d, 3, 0, &layer, &level, &tx, &ty);
	ok &= d.face == 4 && d.size_w == 4 && layer == 4 && level == 2 && tx == 3 && ty == 0;
	check(ok, "narrow: two cube pixels by hand (+X level 0 (0,0), +Z level 2 (3,0))");
	/* ES 2.0 table 3.12: LUMINANCE (L,L,L,1), ALPHA (0,0,0,A),
	 * LUMINANCE_ALPHA (L,L,L,A) */
	uint8_t l8[4], a8[4], la8[4];
	tnc_sampled(TNC_L8, 10, 20, l8);
	tnc_sampled(TNC_A8, 10, 20, a8);
	tnc_sampled(TNC_LA8, 10, 20, la8);
	check(memcmp(l8, (uint8_t[4]){ 10, 10, 10, 255 }, 4) == 0 && memcmp(a8, (uint8_t[4]){ 0, 0, 0, 10 }, 4) == 0 &&
		      memcmp(la8, (uint8_t[4]){ 10, 10, 10, 20 }, 4) == 0,
	      "narrow: L8, A8, LA8 sample as ES 2.0 table 3.12 says");
	check(tnc_controls(print_out), "narrow: tnc_controls pass");

	unsigned applied, ideal = 0;
	for (int f = 0; f < TNC_FORMATS; f++)
		for (int i = 0; i < TNC_DRAWS; i++)
			ideal += nr_verdict((enum tnc_format)f, i, NR_IDEAL, false, &applied);
	check(ideal == TNC_FORMATS * TNC_DRAWS, "narrow: an exact TMU passes all 60 draws");

	static const char *const wrong_names[TNC_WRONG] = { "next x", "next row", "transposed", "next level",
							    "next face", "4-byte rows", "LA8 bytes swapped" };
	for (int w = 0; w < TNC_WRONG; w++) {
		unsigned draws_applied = 0, draws_failed = 0;
		for (int f = 0; f < TNC_FORMATS; f++)
			for (int i = 0; i < TNC_DRAWS; i++) {
				bool v = nr_verdict((enum tnc_format)f, i, NR_WRONG + w, false, &applied);
				if (applied) {
					draws_applied++;
					draws_failed += !v;
				}
			}
		char what[160];
		snprintf(what, sizeof(what), "narrow: %s fails %u of the %u draws it applies to", wrong_names[w],
			 draws_failed, draws_applied);
		check(draws_applied > 0 && draws_failed == draws_applied, what);
	}
	unsigned swz_failed = 0;
	for (int f = 0; f < TNC_FORMATS; f++)
		for (int i = 0; i < TNC_DRAWS; i++)
			swz_failed += !nr_verdict((enum tnc_format)f, i, NR_RGBA8_SWIZZLE, false, &applied);
	check(swz_failed == TNC_FORMATS * TNC_DRAWS, "narrow: RGBA8's swizzle on R8 / RG8 fails all 60 draws");
	check(!nr_verdict(TNC_LA8, 0, NR_IDEAL, true, &applied), "narrow: a read-back one pixel short fails");
	check(nr_verdict(TNC_L8, 3, NR_OFF1, false, &applied) && nr_verdict(TNC_A8, 10, NR_OFF1, false, &applied),
	      "narrow: one step off passes");
	check(!nr_verdict(TNC_L8, 3, NR_OFF2, false, &applied) && !nr_verdict(TNC_A8, 10, NR_OFF2, false, &applied),
	      "narrow: two steps off fail");
}

int main(void)
{
	/* 12 hand pixels; 1 weight claim; 2 control checks; 4 + 6 verdicts; 1 printed;
	 * cube: 3 selection + 2 controls + 2 hand pixels + 5 verdicts;
	 * mip: 2 terms + 2 bounds + 1 levels + 2 controls + 8 verdicts;
	 * cubemip: 2 bounds/levels + 1 controls + 7 verdicts;
	 * cubethin: 2 bounds/split + 1 controls + 3 verdicts;
	 * narrow: 1 population + 1 hand + 1 table 3.12 + 1 controls + 1 ideal + 7 wrong reads + 1 swizzle
	 * + 1 short + 2 tolerance */
	const int planned = 12 + 1 + 2 + (TLC_DRAWS + 6) + 1 + (3 + 2 + 2 + 5) + (2 + 2 + 1 + 2 + 8) + (2 + 1 + 7) +
			    (2 + 1 + 3) + (1 + 1 + 1 + 1 + 1 + TNC_WRONG + 1 + 1 + 2);
	tlc_make_texture();
	part_hand();
	part_weights();
	part_controls();
	part_verdicts();
	part_cube();
	part_mip();
	part_cubemip();
	part_cubethin();
	part_narrow();
	printf("tex_filter_ref_check: %d checks run (planned %d), %d failed\n", checks, planned,
	       failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
