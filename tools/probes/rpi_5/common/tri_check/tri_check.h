/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tri_check.h: classify and check the read-back of one Gouraud triangle over
 * a cleared render target (the Pi 5 probe rpi5_triangle's pixel check; the
 * Pi 4's v3d_triangle checks the same way: inside / outside / edge, tolerance 3).
 *
 * Freestanding C99: no OS calls, no libc, header only (static inline).
 *
 * Pixels are RGBA8888, byte order R,G,B,A at increasing addresses. `stride` is
 * in BYTES (at least width * 4). Row y is at pixels + y * stride; the caller
 * decides which row is the top (window coordinates and the rows must agree).
 * Vertices are in window coordinates, pixel (x, y) has its centre at
 * (x + 0.5, y + 0.5). w = 1 (no perspective), so the colour is the plain
 * barycentric mix of the three vertex colours.
 *
 * Classes, by the signed distance d of the pixel centre to each edge line
 * (positive inside, in pixels):
 *   inside   min d >= margin   expected = barycentric colour, rounded; wrong
 *                              when any channel differs by more than `tol`
 *   outside  any d <= -margin  expected = the clear colour; wrong when any
 *                              channel differs by more than `tol`
 *   edge     otherwise         right when it is within `tol` of the clear
 *                              colour, or every channel lies in the range of
 *                              the three vertex colours (widened by `tol`)
 * Degenerate (zero-area) triangles: every pixel is outside.
 */
#ifndef TRI_CHECK_H
#define TRI_CHECK_H

#include <stddef.h>
#include <stdint.h>

struct tri_check_in {
	const uint8_t *pixels;
	uint32_t width, height;
	size_t stride;          /* bytes per row, >= width * 4 */
	float vx[3], vy[3];     /* window coordinates */
	uint8_t vc[3][4];       /* vertex colours RGBA */
	uint8_t clear[4];       /* clear colour RGBA */
	uint32_t tol;           /* per-channel tolerance (3) */
	float margin;           /* inside/outside margin in pixels (1.0) */
};

struct tri_check_probe {        /* a named pixel and its expected colour */
	const char *name;
	uint32_t x, y;
	uint8_t want[4];
};

struct tri_check_out {
	uint32_t n_inside, n_outside, n_edge;
	uint32_t bad_inside, bad_outside, bad_edge;
	uint32_t max_inside_diff;   /* largest channel difference over inside pixels */
	uint32_t first_bad_x, first_bad_y;  /* first wrong pixel, if any wrong */
	int have_bad;
};

enum { TRI_INSIDE = 0, TRI_OUTSIDE = 1, TRI_EDGE = 2 };

static inline float tri_check_fabs_(float v) { return v < 0 ? -v : v; }

static inline uint32_t tri_check_absdiff_(uint32_t a, uint32_t b)
{
	return a > b ? a - b : b - a;
}

/* Sqrt by Newton iteration (freestanding; inputs are small, positive). */
static inline float tri_check_sqrt_(float v)
{
	float r;
	int i;

	if (v <= 0.0f)
		return 0.0f;
	r = v > 1.0f ? v : 1.0f;
	for (i = 0; i < 30; i++)
		r = 0.5f * (r + v / r);
	return r;
}

/* Twice the signed area of (a, b, p); positive when p is left of a->b. */
static inline float tri_check_edge_(float ax, float ay, float bx, float by,
				    float px, float py)
{
	return (bx - ax) * (py - ay) - (by - ay) * (px - ax);
}

/* Classify the centre (px, py). If bary != NULL it gets the three weights
 * (valid for any class; zeros for a degenerate triangle). */
static inline int tri_check_classify(const struct tri_check_in *t, float px,
				     float py, float bary[3])
{
	float area = tri_check_edge_(t->vx[0], t->vy[0], t->vx[1], t->vy[1],
				     t->vx[2], t->vy[2]);
	float sgn, dmin = 1e30f;
	int i;

	if (bary)
		bary[0] = bary[1] = bary[2] = 0.0f;
	if (tri_check_fabs_(area) < 1e-6f)
		return TRI_OUTSIDE;
	sgn = area > 0 ? 1.0f : -1.0f;
	for (i = 0; i < 3; i++) {
		int a = (i + 1) % 3, b = (i + 2) % 3;
		float e = tri_check_edge_(t->vx[a], t->vy[a], t->vx[b],
					  t->vy[b], px, py);
		float dx = t->vx[b] - t->vx[a], dy = t->vy[b] - t->vy[a];
		float len = tri_check_sqrt_(dx * dx + dy * dy);
		float d = len > 0 ? sgn * e / len : -1e30f;

		if (bary)
			bary[i] = e / area;
		if (d < dmin)
			dmin = d;
	}
	if (dmin >= t->margin)
		return TRI_INSIDE;
	if (dmin <= -t->margin)
		return TRI_OUTSIDE;
	/* Between -margin and margin of the nearest edge: an edge pixel, unless
	 * it is outside the other edges by more than the margin. */
	for (i = 0; i < 3; i++) {
		int a = (i + 1) % 3, b = (i + 2) % 3;
		float e = tri_check_edge_(t->vx[a], t->vy[a], t->vx[b],
					  t->vy[b], px, py);
		float dx = t->vx[b] - t->vx[a], dy = t->vy[b] - t->vy[a];
		float len = tri_check_sqrt_(dx * dx + dy * dy);

		if (len > 0 && sgn * e / len <= -t->margin)
			return TRI_OUTSIDE;
	}
	return TRI_EDGE;
}

/* Expected colour of an inside pixel (barycentric mix, rounded, clamped). */
static inline void tri_check_expect(const struct tri_check_in *t,
				    const float bary[3], uint8_t out[4])
{
	int c, i;

	for (c = 0; c < 4; c++) {
		float v = 0.0f;

		for (i = 0; i < 3; i++)
			v += bary[i] * (float)t->vc[i][c];
		v += 0.5f;
		out[c] = v <= 0.0f ? 0 : v >= 255.0f ? 255 : (uint8_t)v;
	}
}

static inline uint32_t tri_check_maxdiff_(const uint8_t *a, const uint8_t *b)
{
	uint32_t m = 0, d;
	int c;

	for (c = 0; c < 4; c++) {
		d = tri_check_absdiff_(a[c], b[c]);
		if (d > m)
			m = d;
	}
	return m;
}

static inline int tri_check_edge_ok_(const struct tri_check_in *t,
				     const uint8_t *p)
{
	int c, i;

	if (tri_check_maxdiff_(p, t->clear) <= t->tol)
		return 1;
	for (c = 0; c < 4; c++) {
		uint32_t lo = 255, hi = 0;

		for (i = 0; i < 3; i++) {
			if (t->vc[i][c] < lo)
				lo = t->vc[i][c];
			if (t->vc[i][c] > hi)
				hi = t->vc[i][c];
		}
		if ((uint32_t)p[c] + t->tol < lo || p[c] > hi + t->tol)
			return 0;
	}
	return 1;
}

/* Check every pixel of the target. Returns 0 when no pixel is wrong, else 1;
 * -1 for unusable arguments (null pixels, stride < width * 4, no pixels). */
static inline int tri_check_run(const struct tri_check_in *t,
				struct tri_check_out *o)
{
	uint32_t x, y;

	o->n_inside = o->n_outside = o->n_edge = 0;
	o->bad_inside = o->bad_outside = o->bad_edge = 0;
	o->max_inside_diff = 0;
	o->first_bad_x = o->first_bad_y = 0;
	o->have_bad = 0;
	if (!t->pixels || !t->width || !t->height ||
	    t->stride < (size_t)t->width * 4)
		return -1;
	for (y = 0; y < t->height; y++) {
		for (x = 0; x < t->width; x++) {
			const uint8_t *p = t->pixels + (size_t)y * t->stride +
					   (size_t)x * 4;
			float bary[3];
			uint8_t want[4];
			int cls = tri_check_classify(t, (float)x + 0.5f,
						     (float)y + 0.5f, bary);
			int bad = 0;

			if (cls == TRI_INSIDE) {
				uint32_t d;

				tri_check_expect(t, bary, want);
				d = tri_check_maxdiff_(p, want);
				o->n_inside++;
				if (d > o->max_inside_diff)
					o->max_inside_diff = d;
				if (d > t->tol) {
					bad = 1;
					o->bad_inside++;
				}
			} else if (cls == TRI_OUTSIDE) {
				o->n_outside++;
				if (tri_check_maxdiff_(p, t->clear) > t->tol) {
					bad = 1;
					o->bad_outside++;
				}
			} else {
				o->n_edge++;
				if (!tri_check_edge_ok_(t, p)) {
					bad = 1;
					o->bad_edge++;
				}
			}
			if (bad && !o->have_bad) {
				o->have_bad = 1;
				o->first_bad_x = x;
				o->first_bad_y = y;
			}
		}
	}
	return (o->bad_inside | o->bad_outside | o->bad_edge) ? 1 : 0;
}

/* Named probe pixels: returns how many are wrong (outside the target counts
 * as wrong); *first gets the index of the first wrong one (or n). */
static inline uint32_t tri_check_probes(const struct tri_check_in *t,
					const struct tri_check_probe *pr,
					uint32_t n, uint32_t *first)
{
	uint32_t i, bad = 0;

	if (first)
		*first = n;
	for (i = 0; i < n; i++) {
		int wrong = 1;

		if (t->pixels && pr[i].x < t->width && pr[i].y < t->height)
			wrong = tri_check_maxdiff_(t->pixels +
						   (size_t)pr[i].y * t->stride +
						   (size_t)pr[i].x * 4,
						   pr[i].want) > t->tol;
		if (wrong) {
			if (!bad && first)
				*first = i;
			bad++;
		}
	}
	return bad;
}

/* Guard row: `n` bytes of the row at `row` (one past the target, say) must
 * still hold the fill byte. Returns the number of bytes that changed. */
static inline uint32_t tri_check_guard(const uint8_t *pixels, size_t stride,
				       uint32_t row, uint32_t n, uint8_t fill)
{
	const uint8_t *p = pixels + (size_t)row * stride;
	uint32_t i, bad = 0;

	for (i = 0; i < n; i++)
		if (p[i] != fill)
			bad++;
	return bad;
}

#endif /* TRI_CHECK_H */
