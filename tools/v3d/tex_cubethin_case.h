/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_cubethin_case.h: dEQP's thin cube-mipmap cells as a board test, as
 * data and checks a sample includes (docs/online-shaderc/v3d-backend.md,
 * "Cube mipmaps on the board"). tools/v3d/tex_filter_ref_check.c checks it
 * on the host.
 *
 * Why: dEQP-GLES2 texture.mipmap.cube.basic.nearest_nearest fails on the
 * Pi 4 in two full-height columns only: the last column of a 17x128 cell
 * and the first column of the 7x128 cell next to it (qpa8, 8e360630). Each
 * cell draws one whole face (-1..1), and those pixels read the same face
 * one level FINER than dEQP allows. gl_test 133 (tex_cubemip_case.h) draws
 * square quads, and its odd-sized ones also have a 2x2 quad cut by the
 * viewport edge, yet it passes. What 133 lacks is the thin cell: there only
 * one axis sets the LOD, so a quad that loses that axis's difference reads
 * a finer level. This case draws the thin cells, at the pixel parity dEQP
 * has, and says which pixels fail: those whose 2x2 quad the cell cuts, or
 * the others.
 *
 * The scene is tex_cubemip_case.h's: a 64x64 RGBA8 cube, 7 levels, dEQP's
 * colours, uploaded face by face. The read-back surface is 64x64, so dEQP's
 * 128-tall cells become 64 tall; the thin axis, which sets the LOD, is
 * dEQP's:
 *   a 17x64 cell at x 40 (dEQP's cell 14 is 17x128 at x 70): LOD about
 *     1.9, level 2 only. Its last column (x 56, even) is the left half of a
 *     quad whose right half lies outside the cell.
 *   a 7x64 cell at x 57 (dEQP's cell 15 is 7x128 at x 87): LOD about 3.2,
 *     levels 3..4. Its first column (x 57, odd) is the right half of a quad
 *     whose left half lies outside.
 *   the same two cells turned (64x17 at y 40, 64x7 at y 57), for t.
 * Each cell is a viewport with a quad over the whole face, direction
 * major + x * sx + y * ty (tex_cube_case.h's table, x and y -1..1),
 * NEAREST_MIPMAP_NEAREST, mag NEAREST, CLAMP_TO_EDGE, as dEQP's
 * TextureCubeMipmapCase draws a cell.
 *
 * Two arms: the fragment shader samples the interpolated direction as is,
 * then divided by its largest absolute component, as Mesa's
 * nir_normalize_cubemap_coords does before the TMU. If only the first arm
 * fails, the normalization is the fix; if both fail the same way, it is
 * not.
 *
 * The verdict is dEQP's (tex_cubemip_case.h's tcm_pixel_valid): with
 * du/dx = 64 / w and dv/dy = 64 / h, computeLodBoundsFromDerivates
 * (tcuTexLookupVerifier.cpp:2214-2251) takes pMin = the larger of the two
 * and pMax = sqrt2 * the same, which are tex_cubemip_case.h's bounds for a
 * square quad of the thin side over the whole face (h 1).
 *
 * Header only, no OS calls and no libm.
 */
#ifndef TEX_CUBETHIN_CASE_H
#define TEX_CUBETHIN_CASE_H

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "tex_cubemip_case.h"

/* one cell: a viewport, in window pixels from the lower-left corner */
struct tct_cell {
	int x0, y0, w, h;
};

#define TCT_CELLS 4
#define TCT_ARMS 2 /* 0: the direction as is; 1: divided by its largest component */
#define TCT_DRAWS (TCT_ARMS * 6 * TCT_CELLS)

static const struct tct_cell tct_cells[TCT_CELLS] = {
	{ 40, 0, 17, 64 },
	{ 57, 0, 7, 64 },
	{ 0, 40, 64, 17 },
	{ 0, 57, 64, 7 },
};

/* the cell as a tex_cubemip_case.h draw: a square quad of the thin side over
 * the whole face has the same LOD bounds (see the top of the file) */
static inline struct tcm_draw tct_draw(const struct tct_cell *c)
{
	struct tcm_draw d = { false, c->w < c->h ? c->w : c->h, 1.0f };
	return d;
}

/* true when pixel (x, y)'s 2x2 quad has a pixel outside the cell: its
 * partner column (x ^ 1) or row (y ^ 1) lies outside */
static inline bool tct_split(const struct tct_cell *c, int x, int y)
{
	const int px = x ^ 1, py = y ^ 1;
	return px < c->x0 || px >= c->x0 + c->w || py < c->y0 || py >= c->y0 + c->h;
}

struct tct_result {
	unsigned pixels, split, bad_split, bad_whole;
	int first_x, first_y; /* the first invalid pixel */
	uint8_t first[4];
};

static inline void tct_result_init(struct tct_result *r)
{
	memset(r, 0, sizeof(*r));
	r->first_x = r->first_y = -1;
}

/* pixel (x, y) of cell c on `face`, window coordinates */
static inline void tct_check_pixel(int face, const struct tct_cell *c, int x, int y, const uint8_t px[4],
				   struct tct_result *r)
{
	const struct tcm_draw d = tct_draw(c);
	const bool split = tct_split(c, x, y);
	r->pixels++;
	r->split += split;
	if (tcm_pixel_valid(face, &d, px))
		return;
	if (r->bad_split + r->bad_whole == 0) {
		r->first_x = x;
		r->first_y = y;
		memcpy(r->first, px, 4);
	}
	if (split)
		r->bad_split++;
	else
		r->bad_whole++;
}

typedef void (*tct_print_fn)(const char *fmt, ...);

static inline bool tct_verdict(int arm, int face, const struct tct_cell *c, const struct tct_result *r,
			       tct_print_fn print)
{
	static const char *const names[6] = { "+X", "-X", "+Y", "-Y", "+Z", "-Z" };
	const struct tcm_draw d = tct_draw(c);
	float lo, hi;
	int l0, l1;
	tcm_lod_bounds(&d, &lo, &hi);
	tcm_levels(&d, lo, hi, &l0, &l1);
	const unsigned area = (unsigned)(c->w * c->h);
	const bool ok = r->pixels == area && r->bad_split + r->bad_whole == 0;
	const int lo_c = (int)(lo * 100.0f), hi_c = (int)(hi * 100.0f);
	print("tex_cubethin %s %s %dx%d at (%d,%d): LOD %d.%02d..%d.%02d, levels %d..%d; split-quad pixels %u of "
	      "%u invalid, whole-quad pixels %u of %u invalid; %s\n",
	      arm ? "normalized" : "as-is", names[face], c->w, c->h, c->x0, c->y0, lo_c / 100, lo_c % 100,
	      hi_c / 100, hi_c % 100, l0, l1, r->bad_split, r->split, r->bad_whole, r->pixels - r->split,
	      ok ? "PASS" : "FAIL");
	if (r->bad_split + r->bad_whole) {
		int cf, cl, cd;
		tcm_nearest_colour(r->first, &cf, &cl, &cd);
		print("tex_cubethin: first invalid pixel (%d, %d) = %u,%u,%u reads %s level %d (off %d)\n",
		      r->first_x, r->first_y, r->first[0], r->first[1], r->first[2], cf >= 0 ? names[cf] : "?",
		      cl, cd);
	}
	return ok;
}

/* true when `px` is within 4 steps (every channel) of one of face f's
 * levels l0..l1 */
static inline bool tct_close_to_face(const uint8_t px[4], int f, int l0, int l1)
{
	for (int k = l0; k <= l1; k++) {
		uint8_t c[4];
		int worst = 0;
		tcm_colour(f, k, c);
		for (int i = 0; i < 4; i++) {
			int dd = (int)px[i] - (int)c[i];
			if (dd < 0)
				dd = -dd;
			if (dd > worst)
				worst = dd;
		}
		if (worst <= 4)
			return true;
	}
	return false;
}

/* The controls, printed; false when the check could not tell the cases
 * apart. Each cell allows dEQP's levels (2 for the 17-wide cells, 3..4 for
 * the 7-wide ones, as dEQP's cells 14 and 15), accepts each allowed level's
 * colour and rejects the level one finer (what the board's dEQP run read),
 * the level one coarser, and the allowed levels of every other face whose
 * colour dEQP's palette sets apart. Each cell has one line of split-quad
 * pixels, its long side's length. */
static inline bool tct_controls(tct_print_fn print)
{
	static const int want[TCT_CELLS][2] = { { 2, 2 }, { 3, 4 }, { 2, 2 }, { 3, 4 } };
	unsigned accepts = 0, rejects = 0, wrong = 0;
	for (int i = 0; i < TCT_CELLS; i++) {
		const struct tct_cell *c = &tct_cells[i];
		const struct tcm_draw d = tct_draw(c);
		unsigned split = 0;
		float lo, hi;
		int l0, l1;
		tcm_lod_bounds(&d, &lo, &hi);
		tcm_levels(&d, lo, hi, &l0, &l1);
		wrong += l0 != want[i][0] || l1 != want[i][1];
		for (int y = c->y0; y < c->y0 + c->h; y++)
			for (int x = c->x0; x < c->x0 + c->w; x++)
				split += tct_split(c, x, y);
		wrong += split != (unsigned)(c->w > c->h ? c->w : c->h);
		for (int f = 0; f < 6; f++) {
			uint8_t px[4];
			for (int k = l0; k <= l1; k++) {
				tcm_colour(f, k, px);
				accepts++;
				wrong += !tcm_pixel_valid(f, &d, px);
			}
			tcm_colour(f, l0 - 1, px);
			wrong += tcm_pixel_valid(f, &d, px);
			tcm_colour(f, l1 + 1, px);
			wrong += tcm_pixel_valid(f, &d, px);
			rejects += 2;
			/* every other face's allowed levels, where dEQP's palette
			 * tells them apart: -X, +Y and -Z at level 3 are within 3
			 * steps of one another, inside the threshold (3.98) */
			for (int g = 0; g < 6; g++)
				for (int k = l0; k <= l1 && g != f; k++) {
					tcm_colour(g, k, px);
					if (tct_close_to_face(px, f, l0, l1))
						continue;
					rejects++;
					wrong += tcm_pixel_valid(f, &d, px);
				}
		}
	}
	const bool ok = wrong == 0;
	print("tex_cubethin controls: %u reference colours accepted, %u wrong colours rejected, %u exceptions; "
	      "%s\n",
	      accepts, rejects, wrong, ok ? "the check can tell them apart" : "FAIL");
	return ok;
}

#endif
