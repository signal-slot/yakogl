/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The read-back check of v3d_triangle (see main.c), in plain C so that
 * host/tri_check_test.c can show on the host that it passes a correct picture
 * and fails each kind of wrong one. It models what the picture must be; it
 * draws nothing on the board.
 */
#ifndef TRI_CHECK_H
#define TRI_CHECK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "v3d_frame.h"

#define EDGE_PX   1.0f /* the band around each edge that is not classified */
#define COLOR_TOL 3    /* of 255, per channel: a stated guess until the board measures it */

/* The triangle in normalized device coordinates, and its vertex colours,
 * laid out as the vertex buffer: vec4 position, vec4 colour. */
static const float tri_vertices[3][8] = {
	/* x, y, z, w, r, g, b, a */
	{ -0.5f, -0.5f, 0.0f, 1.0f, 1.0f, 0.0f, 0.0f, 1.0f },
	{ 0.5f, -0.5f, 0.0f, 1.0f, 0.0f, 1.0f, 0.0f, 1.0f },
	{ 0.0f, 0.5f, 0.0f, 1.0f, 0.0f, 0.0f, 1.0f, 1.0f },
};

/*
 * What the program does to the vertex buffer's values: the position plus
 * `offset` (x and y; z and w must stay 0, so w is still 1 and the divide a
 * no-op) and the colour times `scale` plus `bias`, per channel. The plain
 * sample's shaders are the identity; the uniform variant's (es100/utri) take
 * the three from uniform storage.
 */
struct tri_xform {
	float offset[4], scale[4], bias[4];
};

static const struct tri_xform tri_identity = {
	{ 0.0f, 0.0f, 0.0f, 0.0f }, { 1.0f, 1.0f, 1.0f, 1.0f }, { 0.0f, 0.0f, 0.0f, 0.0f }
};

/*
 * The uniform variant's values: main.c writes them into uniform storage
 * (u_offset, u_scale, u_bias) and the check expects their effect. Every
 * channel differs from the identity and from every other channel, and each
 * value is exact in float16, so a word read from the wrong place, or not
 * read at all (0), changes the picture.
 */
static const struct tri_xform tri_uniform_values = {
	{ 0.25f, -0.125f, 0.0f, 0.0f },   /* u_offset */
	{ 0.5f, 0.75f, 0.25f, 1.0f },     /* u_scale */
	{ 0.25f, 0.0625f, 0.5f, 0.0f },   /* u_bias */
};

/*
 * The if variant (es100/tri_if.frag): `if (v_color.r > 0.5) c.g = 1.0; else
 * c.b = 0.25;` on the interpolated colour, with no uniforms. A pixel whose
 * interpolated red is within TRI_IF_BAND of 0.5 may land on either side (the
 * GPU's interpolation and ours differ in the last bits), so it is checked
 * against both; every other pixel against its own side only.
 */
#define TRI_IF_BAND 0.01f

/*
 * The int variant (es100/tri_int.frag), with no uniforms:
 *   i = int(r * 4.0); j = -i * 3 + 7;
 *   colour = (float(j + 5) / 12.0,
 *             float(i == 1) * 0.5 + float(i != 2) * 0.25 + 0.125,
 *             (b < 0.5 ? 0.25 : 0.75) + sign(g - 0.5) * 0.125, 1.0)
 * on the interpolated r, g, b. A pixel within TRI_INT_BAND of a threshold
 * (r * 4 at an integer, g or b at 0.5) may land on either side of it, so
 * every side it is near is accepted there; every other pixel is checked
 * against its own side only.
 */
#define TRI_INT_BAND 0.01f

/*
 * The discard variant (es100/tri_discard.frag): `if (g > 0.25 && b < 0.25)
 * discard;` then the interpolated colour. A discarded pixel keeps what the
 * target held: the clear colour. Within TRI_DISCARD_BAND of either threshold
 * both outcomes are accepted.
 */
#define TRI_DISCARD_BAND 0.01f

/*
 * The gl_FragCoord variant (es100/tri_fragcoord.frag): (x / 1280, y / 512,
 * the interpolated b, 1) in GL window coordinates, whose y grows upward:
 * memory row y of a target of height h has window y = h - (y + 0.5). The
 * wrong flip would give y + 0.5. A pixel whose two answers are within
 * COLOR_TOL of each other (the rows around h / 2) cannot tell them apart and
 * is counted in the band; every other inside pixel must match the right one,
 * and the check requires such pixels, and counts any that match the wrong one.
 */
#define TRI_FRAGCOORD_W 1280.0f
#define TRI_FRAGCOORD_H 512.0f
#define TRI_MAX_CANDIDATES 12

/* Window-space vertices and the edge lengths of the triangle. */
struct raster {
	struct tri_xform xf;
	bool if_variant;
	bool int_variant;
	bool discard_variant;
	bool fragcoord_variant;
	float fb_h;     /* the fragcoord variant: the target's height */
	uint32_t clear; /* the discard variant: what a discarded pixel shows */
	float x[3], y[3];
	float area;   /* twice the signed area */
	float len[3]; /* the length of the edge opposite vertex i */
};

static inline float tri_sqrt(float v)
{
	/* Newton's method; v > 0 here (an edge length squared) */
	float r = v > 1.0f ? v : 1.0f;
	for (int i = 0; i < 40; i++)
		r = 0.5f * (r + v / r);
	return r;
}

/* The viewport transform gallium hands the V3D (window = ndc * scale +
 * translate); w is 1 at every vertex, so the perspective divide is a no-op. */
static inline void raster_setup(const struct v3d_viewport *vp, const struct tri_xform *xf,
				struct raster *r)
{
	/* Every field starts zeroed, the variant flags included, so a field
	 * added later cannot be left to whatever the stack held (fragcoord_variant
	 * and fb_h were, once, and the host test's own picture failed). */
	*r = (struct raster){ .xf = *xf };
	for (int i = 0; i < 3; i++) {
		r->x[i] = (tri_vertices[i][0] + xf->offset[0]) * vp->scale[0] + vp->translate[0];
		r->y[i] = (tri_vertices[i][1] + xf->offset[1]) * vp->scale[1] + vp->translate[1];
	}
	r->area = (r->x[1] - r->x[0]) * (r->y[2] - r->y[0]) -
		  (r->x[2] - r->x[0]) * (r->y[1] - r->y[0]);
	for (int i = 0; i < 3; i++) {
		int a = (i + 1) % 3, b = (i + 2) % 3;
		float dx = r->x[b] - r->x[a], dy = r->y[b] - r->y[a];
		r->len[i] = tri_sqrt(dx * dx + dy * dy);
	}
}

/* Barycentric weights of (px, py), and its signed distance in pixels from
 * the nearest edge (positive inside). With w = 1 at every vertex the
 * perspective-correct interpolation is the linear one. */
static inline float raster_at(const struct raster *r, float px, float py, float lambda[3])
{
	float dmin = 1e30f;
	float abs_area = r->area < 0 ? -r->area : r->area;
	for (int i = 0; i < 3; i++) {
		int a = (i + 1) % 3, b = (i + 2) % 3;
		float e = (r->x[b] - r->x[a]) * (py - r->y[a]) - (r->y[b] - r->y[a]) * (px - r->x[a]);
		lambda[i] = e / r->area;
		float d = lambda[i] * abs_area / r->len[i];
		if (d < dmin)
			dmin = d;
	}
	return dmin;
}

static inline uint32_t unorm8(float c)
{
	if (c <= 0.0f)
		return 0;
	if (c >= 1.0f)
		return 255;
	return (uint32_t)(c * 255.0f + 0.5f);
}

/* The interpolated red, the value the if variant tests. */
static inline float interp_red(const float lambda[3])
{
	float c = 0;
	for (int i = 0; i < 3; i++)
		c += lambda[i] * tri_vertices[i][4];
	return c;
}

/* The expected colour as R8G8B8A8 (R in the low byte): the interpolated
 * colour, scaled and biased; for the if variant, the arm `then_side` (1: the
 * then arm, 0: the else arm) applied. */
static inline uint32_t expected_side(const struct raster *r, const float lambda[3], int then_side)
{
	float c[4];
	for (int ch = 0; ch < 4; ch++) {
		c[ch] = 0;
		for (int i = 0; i < 3; i++)
			c[ch] += lambda[i] * tri_vertices[i][4 + ch];
		c[ch] = c[ch] * r->xf.scale[ch] + r->xf.bias[ch];
	}
	if (r->if_variant) {
		if (then_side)
			c[1] = 1.0f;
		else
			c[2] = 0.25f;
	}
	uint32_t v = 0;
	for (int ch = 0; ch < 4; ch++)
		v |= unorm8(c[ch]) << (8 * ch);
	return v;
}

/* The expected colour on the pixel's own side of the if. */
static inline uint32_t expected_rgba8(const struct raster *r, const float lambda[3])
{
	return expected_side(r, lambda, interp_red(lambda) > 0.5f);
}

/* For the if variant near its threshold: the other side's colour, which is
 * also accepted there; otherwise the same as expected_rgba8. */
static inline uint32_t expected_alt(const struct raster *r, const float lambda[3])
{
	float red = interp_red(lambda);
	if (r->if_variant && red > 0.5f - TRI_IF_BAND && red < 0.5f + TRI_IF_BAND)
		return expected_side(r, lambda, !(red > 0.5f));
	return expected_rgba8(r, lambda);
}

/* The interpolated colour channel `ch` (0 r, 1 g, 2 b, 3 a). */
static inline float interp_ch(const float lambda[3], int ch)
{
	float c = 0;
	for (int k = 0; k < 3; k++)
		c += lambda[k] * tri_vertices[k][4 + ch];
	return c;
}

/* The int variant's colour for one choice of sides: `i` = int(r * 4.0),
 * `gs` = sign(g - 0.5) (-1, 0 or 1), `b_low` = (b < 0.5). */
static inline uint32_t int_colour(int i, int gs, int b_low)
{
	int j = -i * 3 + 7;
	float c[4] = {
		(float)(j + 5) / 12.0f,
		(i == 1 ? 1.0f : 0.0f) * 0.5f + (i != 2 ? 1.0f : 0.0f) * 0.25f + 0.125f,
		(b_low ? 0.25f : 0.75f) + (float)gs * 0.125f,
		1.0f,
	};
	uint32_t v = 0;
	for (int ch = 0; ch < 4; ch++)
		v |= unorm8(c[ch]) << (8 * ch);
	return v;
}

/* The int variant's pixel's own sides: i, sign(g - 0.5), b < 0.5. */
static inline void int_sides(const float lambda[3], int *i, int *gs, int *b_low)
{
	float x = interp_ch(lambda, 0) * 4.0f, dg = interp_ch(lambda, 1) - 0.5f;
	*i = (int)x; /* x >= 0: int() truncates toward zero, as GLSL's */
	*gs = dg > 0 ? 1 : dg < 0 ? -1 : 0;
	*b_low = interp_ch(lambda, 2) < 0.5f;
}

/* Every colour a pixel may correctly have: its own sides first, and near a
 * threshold every side it is near. Returns the count; *near says whether
 * the pixel is in some band. Plain and uniform pixels have one candidate,
 * the if variant's one or two. */
/* The fragcoord variant's colour at pixel centre (px, py) of memory, with
 * the window flip right (`right`) or wrong. */
static inline uint32_t fragcoord_colour(const struct raster *r, const float lambda[3], float px,
					float py, bool right)
{
	float wy = right ? r->fb_h - py : py;
	uint32_t base = expected_rgba8(r, lambda);
	return (base & 0xffff0000u) | unorm8(px / TRI_FRAGCOORD_W) | (unorm8(wy / TRI_FRAGCOORD_H) << 8);
}

/* (px, py): the pixel centre in memory coordinates (row 0 at the top). */
static inline int candidates(const struct raster *r, const float lambda[3], float px, float py,
			     uint32_t out[TRI_MAX_CANDIDATES], bool *near)
{
	*near = false;
	if (r->fragcoord_variant) {
		out[0] = fragcoord_colour(r, lambda, px, py, true);
		return 1;
	}
	if (r->discard_variant) {
		float dg = interp_ch(lambda, 1) - 0.25f, db = interp_ch(lambda, 2) - 0.25f;
		bool gone = dg > 0 && db < 0;
		uint32_t drawn = expected_rgba8(r, lambda);
		out[0] = gone ? r->clear : drawn;
		if ((dg > -TRI_DISCARD_BAND && dg < TRI_DISCARD_BAND) ||
		    (db > -TRI_DISCARD_BAND && db < TRI_DISCARD_BAND)) {
			*near = true;
			out[1] = gone ? drawn : r->clear;
			return 2;
		}
		return 1;
	}
	if (!r->int_variant) {
		out[0] = expected_rgba8(r, lambda);
		uint32_t alt = expected_alt(r, lambda);
		if (alt == out[0])
			return 1;
		*near = true;
		out[1] = alt;
		return 2;
	}
	int i, gs, b_low;
	int_sides(lambda, &i, &gs, &b_low);
	float x = interp_ch(lambda, 0) * 4.0f, dg = interp_ch(lambda, 1) - 0.5f;
	float db = interp_ch(lambda, 2) - 0.5f;
	int is[2] = { i, i }, ni = 1;
	float fr = x - (float)i;
	if (fr < 4.0f * TRI_INT_BAND && i > 0)
		is[ni++] = i - 1;
	else if (fr > 1.0f - 4.0f * TRI_INT_BAND)
		is[ni++] = i + 1;
	int gss[3] = { gs, gs, gs }, ng = 1;
	if (dg > -TRI_INT_BAND && dg < TRI_INT_BAND) {
		gss[0] = gs;
		gss[1] = gs == 1 ? -1 : 1;
		gss[2] = gs == 0 ? -1 : 0;
		ng = 3;
	}
	int bs[2] = { b_low, !b_low }, nb = (db > -TRI_INT_BAND && db < TRI_INT_BAND) ? 2 : 1;
	*near = ni > 1 || ng > 1 || nb > 1;
	int n = 0;
	for (int a = 0; a < ni; a++)
		for (int b = 0; b < ng; b++)
			for (int c = 0; c < nb; c++)
				out[n++] = int_colour(is[a], gss[b], bs[c]);
	return n;
}

/* The largest per-channel difference of two RGBA8 values. */
static inline uint32_t max_diff(uint32_t a, uint32_t b)
{
	uint32_t m = 0;
	for (int ch = 0; ch < 4; ch++) {
		int d = (int)((a >> (8 * ch)) & 0xff) - (int)((b >> (8 * ch)) & 0xff);
		uint32_t ad = (uint32_t)(d < 0 ? -d : d);
		if (ad > m)
			m = ad;
	}
	return m;
}

struct tri_result {
	uint32_t total, n_in, n_out, n_edge;
	uint32_t bad_in, bad_out, bad_edge, edge_drawn, worst_in;
	/* the if variant: inside pixels on each side, and in the band */
	uint32_t n_then, n_else, n_band;
	/* the int variant: inside pixels (out of every band) per i = 0..3, on
	 * each side of g = 0.5 and of b = 0.5 */
	uint32_t n_int_i[4], n_int_g[2], n_int_b[2];
	/* the discard variant: inside pixels (out of the band) discarded, drawn */
	uint32_t n_discarded, n_drawn;
	/* the fragcoord variant: inside pixels out of the band that match the
	 * right flip, that match the wrong one */
	uint32_t n_flip_right, n_flip_wrong;
	bool counted; /* the classes cover the target and none of in/out is empty */
	bool have_first;
	uint32_t first_x, first_y, first_got, first_want;
};

/* Classify and check every pixel of a width x height RGBA8 image. */
static inline void tri_check(const struct raster *r, const uint8_t *img, uint32_t pitch,
			     uint32_t width, uint32_t height, uint32_t clear_rgba8,
			     struct tri_result *res)
{
	*res = (struct tri_result){ .total = width * height };
	for (uint32_t y = 0; y < height; y++) {
		const uint32_t *p = (const uint32_t *)(img + (size_t)y * pitch);
		for (uint32_t x = 0; x < width; x++) {
			float lambda[3];
			float d = raster_at(r, x + 0.5f, y + 0.5f, lambda);
			uint32_t cand[TRI_MAX_CANDIDATES];
			bool near;
			int nc = candidates(r, lambda, x + 0.5f, y + 0.5f, cand, &near);
			uint32_t want = cand[0], got = p[x];
			uint32_t diff = 0xffffffffu;
			for (int k = 0; k < nc; k++) {
				uint32_t dk = max_diff(got, cand[k]);
				if (dk < diff)
					diff = dk;
			}
			bool bad;
			if (d >= EDGE_PX) {
				res->n_in++;
				if (r->if_variant) {
					if (near)
						res->n_band++;
					else if (interp_red(lambda) > 0.5f)
						res->n_then++;
					else
						res->n_else++;
				}
				if (r->fragcoord_variant) {
					uint32_t wrong = fragcoord_colour(r, lambda, x + 0.5f, y + 0.5f, false);
					if (max_diff(cand[0], wrong) <= COLOR_TOL)
						res->n_band++;
					else if (max_diff(got, cand[0]) <= COLOR_TOL)
						res->n_flip_right++;
					else if (max_diff(got, wrong) <= COLOR_TOL)
						res->n_flip_wrong++;
				}
				if (r->discard_variant) {
					if (near)
						res->n_band++;
					else if (interp_ch(lambda, 1) > 0.25f && interp_ch(lambda, 2) < 0.25f)
						res->n_discarded++;
					else
						res->n_drawn++;
				}
				if (r->int_variant) {
					int i, gs, b_low;
					int_sides(lambda, &i, &gs, &b_low);
					if (near)
						res->n_band++;
					else {
						if (i >= 0 && i < 4)
							res->n_int_i[i]++;
						res->n_int_g[gs > 0]++;
						res->n_int_b[b_low]++;
					}
				}
				if (diff > res->worst_in)
					res->worst_in = diff;
				bad = diff > COLOR_TOL;
				res->bad_in += bad;
			} else if (d <= -EDGE_PX) {
				res->n_out++;
				want = clear_rgba8;
				bad = got != clear_rgba8;
				res->bad_out += bad;
			} else {
				res->n_edge++;
				bool drawn = diff <= COLOR_TOL;
				res->edge_drawn += drawn;
				bad = !drawn && got != clear_rgba8;
				res->bad_edge += bad;
			}
			if (bad && !res->have_first) {
				res->have_first = true;
				res->first_x = x;
				res->first_y = y;
				res->first_got = got;
				res->first_want = want;
			}
		}
	}
	res->counted = res->n_in + res->n_out + res->n_edge == res->total && res->n_in > 0 &&
		       res->n_out > 0;
	/* the if variant proves both arms only when both sides were drawn */
	if (r->if_variant && (res->n_then == 0 || res->n_else == 0))
		res->counted = false;
	/* the int variant proves each operation only when every band of i and
	 * both sides of each comparison were drawn out of every band */
	/* the discard variant proves the discard only with pixels of both kinds */
	if (r->discard_variant && (res->n_discarded == 0 || res->n_drawn == 0))
		res->counted = false;
	/* the fragcoord variant proves the flip only with pixels out of the band */
	if (r->fragcoord_variant && res->n_flip_right + res->n_flip_wrong == 0)
		res->counted = false;
	if (r->int_variant) {
		for (int k = 0; k < 4; k++)
			res->counted &= res->n_int_i[k] > 0;
		res->counted &= res->n_int_g[0] > 0 && res->n_int_g[1] > 0;
		res->counted &= res->n_int_b[0] > 0 && res->n_int_b[1] > 0;
	}
}

static inline bool tri_result_ok(const struct tri_result *res)
{
	return res->counted && !res->bad_in && !res->bad_out && !res->bad_edge;
}

#endif
