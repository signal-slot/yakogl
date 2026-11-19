/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host test of tri_check.h. Prints "tri_check: N checks, 0 failed". */
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "tri_check.h"

static int checks, failed;

#define CHECK(c) do { checks++; if (!(c)) { failed++; \
	printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); } } while (0)

#define W 64
#define H 64

/* the board's Mesa triangle (tools/v3d/gl_ref.py) in window coordinates, y down */
static struct tri_check_in make(const uint8_t *px, size_t stride)
{
	struct tri_check_in t;

	memset(&t, 0, sizeof t);
	t.pixels = px;
	t.width = W;
	t.height = H;
	t.stride = stride;
	t.vx[0] = 8;  t.vy[0] = 56;
	t.vx[1] = 56; t.vy[1] = 56;
	t.vx[2] = 32; t.vy[2] = 8;
	t.vc[0][0] = 255; t.vc[0][3] = 255;
	t.vc[1][1] = 255; t.vc[1][3] = 255;
	t.vc[2][2] = 255; t.vc[2][3] = 255;
	t.clear[2] = 255; t.clear[3] = 255;
	t.tol = 3;
	t.margin = 1.0f;
	return t;
}

/* Independent renderer: doubles, edge pixels get the clear colour. */
static void render(const struct tri_check_in *t, uint8_t *px)
{
	uint32_t x, y;
	int c;

	for (y = 0; y < t->height; y++)
		for (x = 0; x < t->width; x++) {
			uint8_t *p = px + y * t->stride + x * 4;
			double X = x + 0.5, Y = y + 0.5;
			double x0 = t->vx[0], y0 = t->vy[0], x1 = t->vx[1],
			       y1 = t->vy[1], x2 = t->vx[2], y2 = t->vy[2];
			double den = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2);
			double l0 = ((y1 - y2) * (X - x2) + (x2 - x1) * (Y - y2)) / den;
			double l1 = ((y2 - y0) * (X - x2) + (x0 - x2) * (Y - y2)) / den;
			double l2 = 1 - l0 - l1;
			int cls = tri_check_classify(t, (float)X, (float)Y, NULL);

			for (c = 0; c < 4; c++) {
				double v = cls == TRI_INSIDE ?
					l0 * t->vc[0][c] + l1 * t->vc[1][c] +
					l2 * t->vc[2][c] : t->clear[c];
				p[c] = (uint8_t)floor(v + 0.5);
			}
		}
}

static void run(const struct tri_check_in *t, struct tri_check_out *o, int want)
{
	CHECK(tri_check_run(t, o) == want);
}

/* First pixel of a class; for inside, the one at (32, 40) (a mix of all three). */
static int pick(const struct tri_check_in *t, int cls, uint32_t *x, uint32_t *y)
{
	uint32_t i, j;

	if (cls == TRI_INSIDE) {
		*x = 32;
		*y = 40;
		return tri_check_classify(t, 32.5f, 40.5f, NULL) == TRI_INSIDE;
	}
	for (j = 0; j < t->height; j++)
		for (i = 0; i < t->width; i++)
			if (tri_check_classify(t, i + 0.5f, j + 0.5f, NULL) == cls) {
				*x = i;
				*y = j;
				return 1;
			}
	return 0;
}

static void single_errors(const struct tri_check_in *t, uint8_t *px)
{
	static const int cls[3] = { TRI_INSIDE, TRI_OUTSIDE, TRI_EDGE };
	int k;

	for (k = 0; k < 3; k++) {
		uint32_t x = 0, y = 0;
		uint8_t save[4], *p;
		struct tri_check_out o;

		CHECK(pick(t, cls[k], &x, &y));
		p = px + y * t->stride + x * 4;
		memcpy(save, p, 4);
		/* a colour no class accepts */
		p[0] = 128; p[1] = 128; p[2] = 128; p[3] = 0;
		run(t, &o, 1);
		CHECK((k == 0 ? o.bad_inside : k == 1 ? o.bad_outside : o.bad_edge) == 1);
		CHECK(o.bad_inside + o.bad_outside + o.bad_edge == 1);
		CHECK(o.have_bad && o.first_bad_x == x && o.first_bad_y == y);
		memcpy(p, save, 4);
	}
}

int main(void)
{
	static uint8_t img[W * 4 * (H + 1)], big[(W * 4 + 40) * (H + 2)];
	struct tri_check_in t = make(img, W * 4);
	struct tri_check_out o;
	uint32_t x, y, first;
	uint8_t *p;
	size_t stride2 = W * 4 + 40;

	memset(img, 0xA5, sizeof img);
	render(&t, img);
	run(&t, &o, 0);
	CHECK(o.n_inside > 500 && o.n_outside > 1000 && o.n_edge > 50);
	CHECK(o.n_inside + o.n_outside + o.n_edge == W * H);
	CHECK(o.bad_inside == 0 && o.bad_outside == 0 && o.bad_edge == 0);
	CHECK(o.max_inside_diff <= 1);

	/* named probes */
	{
		struct tri_check_probe pr[3] = {
			{ "out", 2, 2, { 0, 0, 255, 255 } },
			{ "in", 32, 40, { 0, 0, 0, 0 } },
			{ "out2", 60, 30, { 0, 0, 255, 255 } },
		};

		memcpy(pr[1].want, img + 40 * t.stride + 32 * 4, 4);
		CHECK(tri_check_probes(&t, pr, 3, &first) == 0 && first == 3);
		img[40 * t.stride + 32 * 4 + 1] ^= 4;   /* may be 4 off: wrong */
		CHECK(tri_check_probes(&t, pr, 3, &first) == 1 && first == 1);
		img[40 * t.stride + 32 * 4 + 1] ^= 4;
		pr[2].x = W;                           /* outside the target */
		CHECK(tri_check_probes(&t, pr, 3, &first) == 1 && first == 2);
	}

	/* a guard row after the target */
	memset(img + H * t.stride, 0xA5, W * 4);
	CHECK(tri_check_guard(img, t.stride, H, W * 4, 0xA5) == 0);
	img[H * t.stride + 7] = 0;
	CHECK(tri_check_guard(img, t.stride, H, W * 4, 0xA5) == 1);
	img[H * t.stride + 7] = 0xA5;

	single_errors(&t, img);

	/* tolerance: 3 passes, 4 fails, in an inside and an outside pixel */
	CHECK(pick(&t, TRI_INSIDE, &x, &y));
	p = img + y * t.stride + x * 4;
	{
		uint8_t save = p[0];

		p[0] = save >= 4 ? save - 3 : save + 3;
		run(&t, &o, 0);
		CHECK(o.bad_inside == 0);
		p[0] = save >= 5 ? save - 5 : save + 5;
		run(&t, &o, 1);
		CHECK(o.bad_inside == 1);
		p[0] = save;
	}
	CHECK(pick(&t, TRI_OUTSIDE, &x, &y));
	p = img + y * t.stride + x * 4;
	{
		uint8_t save = p[1];

		p[1] = save + 3;
		run(&t, &o, 0);
		p[1] = save + 4;
		run(&t, &o, 1);
		CHECK(o.bad_outside == 1);
		p[1] = save;
	}
	/* edge: a colour in the vertex range is right, one outside is wrong */
	CHECK(pick(&t, TRI_EDGE, &x, &y));
	p = img + y * t.stride + x * 4;
	{
		uint8_t save[4];

		memcpy(save, p, 4);
		p[0] = 100; p[1] = 100; p[2] = 100; p[3] = 255;
		run(&t, &o, 0);
		p[3] = 0;       /* alpha below the vertices' range by more than tol */
		run(&t, &o, 1);
		CHECK(o.bad_edge == 1);
		memcpy(p, save, 4);
	}
	run(&t, &o, 0);

	/* a stride larger than the width, the padding holding junk */
	{
		struct tri_check_in t2 = make(big, stride2);

		memset(big, 0x5A, sizeof big);
		render(&t2, big);
		run(&t2, &o, 0);
		CHECK(o.n_inside + o.n_outside + o.n_edge == W * H);
		CHECK(tri_check_guard(big, stride2, H, W * 4, 0x5A) == 0);
		big[3 * stride2 + 5 * 4] ^= 0xFF;       /* pixel (5,3), outside */
		run(&t2, &o, 1);
		CHECK(o.bad_outside == 1 && o.first_bad_x == 5 && o.first_bad_y == 3);
		t2.stride = W * 4 - 1;                  /* refused */
		CHECK(tri_check_run(&t2, &o) == -1);
	}

	/* a degenerate triangle: everything is outside */
	{
		struct tri_check_in t3 = make(img, W * 4);

		t3.vy[2] = 56;
		for (y = 0; y < H; y++)
			for (x = 0; x < W; x++)
				memcpy(img + y * t3.stride + x * 4, t3.clear, 4);
		run(&t3, &o, 0);
		CHECK(o.n_outside == W * H);
	}

	printf("tri_check: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
