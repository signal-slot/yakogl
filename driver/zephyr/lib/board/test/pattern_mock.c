// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * Host test of driver/zephyr/lib/board/rpi5_scanout/pattern.h: fill then check
 * is clean for each format, size and a padded pitch; injected faults (one
 * flipped byte, a swapped R/B, a wrong pitch, a wrong row) are each found at
 * the right x, y; the sparse check; the split layout; a band filled alone.
 * Built by tools/v3d/check71.d/pattern.sh.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "pattern.h"

static int total;
static int failed;

#define CHECK(c)                                                                         \
	do {                                                                             \
		total++;                                                                 \
		if (!(c)) {                                                              \
			failed++;                                                        \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);              \
		}                                                                        \
	} while (0)

static const enum pattern_fmt fmts[] = { PATTERN_XRGB8888, PATTERN_XBGR8888, PATTERN_RGB565 };
static const struct { uint32_t w, h, pad; } sizes[] = {
	{ 1920, 1080, 0 }, { 640, 480, 0 }, { 640, 480, 192 }, { 1920, 1080, 256 }, { 37, 21, 12 },
};

struct buf {
	uint8_t *p;
	uint32_t w, h, pitch, bpp;
	enum pattern_fmt fmt;
};

static struct buf buf_new(uint32_t w, uint32_t h, uint32_t pad, enum pattern_fmt fmt)
{
	struct buf b = { .w = w, .h = h, .fmt = fmt };
	uint32_t bp = pattern_bytes_per_pixel(fmt);

	b.bpp = bp * 8u;
	b.pitch = w * bp + pad;
	b.p = calloc((size_t)b.pitch * h + (size_t)h * 8u, 1); /* slack: a longer pitch reads further */
	if (!b.p) {
		abort();
	}
	return b;
}

static int chk(const struct buf *b, uint32_t pitch, uint32_t step, struct pattern_report *r)
{
	return pattern_check(b->p, b->w, b->h, pitch, b->bpp, b->fmt, step, r);
}

static uint8_t *at(const struct buf *b, uint32_t x, uint32_t y)
{
	return b->p + (size_t)y * b->pitch + (size_t)x * (b->bpp / 8u);
}

static void test_clean(void)
{
	for (unsigned f = 0; f < 3; f++) {
		for (unsigned s = 0; s < sizeof(sizes) / sizeof(sizes[0]); s++) {
			struct buf b = buf_new(sizes[s].w, sizes[s].h, sizes[s].pad, fmts[f]);
			struct pattern_report r, r2;

			CHECK(pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt) == 0);
			CHECK(chk(&b, b.pitch, 1, &r) == 0);
			CHECK(r.mismatches == 0 && r.checked == b.w * b.h);
			CHECK(chk(&b, b.pitch, 1, &r2) == 0 && r2.checksum == r.checksum);
			/* the padding is not touched */
			if (sizes[s].pad) {
				CHECK(at(&b, b.w, 0)[0] == 0);
			}
			free(b.p);
		}
	}
}

static void test_args(void)
{
	struct buf b = buf_new(64, 8, 0, PATTERN_XRGB8888);
	struct pattern_report r;

	CHECK(pattern_fill(b.p, 64, 8, b.pitch, 16, PATTERN_XRGB8888) == -1);
	CHECK(pattern_fill(b.p, 64, 8, 64 * 4 - 1, 32, PATTERN_XRGB8888) == -1);
	CHECK(pattern_check(b.p, 64, 8, b.pitch, 32, PATTERN_RGB565, 1, &r) == -1);
	CHECK(pattern_check(NULL, 64, 8, b.pitch, 32, PATTERN_XRGB8888, 1, &r) == -1);
	CHECK(!strcmp(pattern_fmt_name(PATTERN_XRGB8888), "XRGB8888"));
	CHECK(!strcmp(pattern_fmt_name(PATTERN_XBGR8888), "XBGR8888"));
	CHECK(!strcmp(pattern_fmt_name(PATTERN_RGB565), "RGB565"));
	CHECK(!strcmp(pattern_fmt_name((enum pattern_fmt)9), "?"));
	free(b.p);
}

/* The bars: 8 distinct bar colours in the top bits, the formats differ. */
static void test_pixels(void)
{
	uint32_t w = 640;

	CHECK(pattern_pixel(0, 0, w, PATTERN_XRGB8888) >> 24 == 0);
	/* bar 0 is white: both top bits set in each channel */
	CHECK((pattern_pixel(0, 5, w, PATTERN_XRGB8888) & 0xc0c0c0u) == 0xc0c0c0u);
	/* bar 7 is black */
	CHECK((pattern_pixel(w - 1, 5, w, PATTERN_XRGB8888) & 0xc0c0c0u) == 0);
	/* bar 5 is red: XRGB has it in bits 23..16, XBGR in bits 7..0 */
	CHECK((pattern_pixel(w * 5 / 8, 3, w, PATTERN_XRGB8888) & 0xc0c0c0u) == 0xc00000u);
	CHECK((pattern_pixel(w * 5 / 8, 3, w, PATTERN_XBGR8888) & 0xc0c0c0u) == 0x0000c0u);
	CHECK((pattern_pixel(w * 5 / 8, 3, w, PATTERN_RGB565) & 0xffffu) >> 14 == 3u);
	/* the hash tells neighbours apart */
	CHECK(pattern_pixel(10, 4, w, PATTERN_XRGB8888) != pattern_pixel(11, 4, w, PATTERN_XRGB8888));
	CHECK(pattern_pixel(10, 4, w, PATTERN_XRGB8888) != pattern_pixel(10, 5, w, PATTERN_XRGB8888));
}

static void test_byte_flip(void)
{
	for (unsigned f = 0; f < 3; f++) {
		struct buf b = buf_new(640, 480, 64, fmts[f]);
		uint32_t bp = b.bpp / 8u;

		pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
		for (uint32_t k = 0; k < bp; k++) {
			struct pattern_report r;
			uint32_t x = 301 + k, y = 222 + k;

			at(&b, x, y)[k] ^= 0x01;
			CHECK(chk(&b, b.pitch, 1, &r) == 1);
			CHECK(r.mismatches == 1 && r.first_x == x && r.first_y == y);
			CHECK(r.expected == pattern_pixel(x, y, b.w, b.fmt));
			CHECK((r.expected ^ r.got) == (1u << (8u * k)));
			at(&b, x, y)[k] ^= 0x01;
		}
		/* the last pixel of the last row */
		struct pattern_report r;

		at(&b, b.w - 1, b.h - 1)[0] ^= 0x80;
		CHECK(chk(&b, b.pitch, 1, &r) == 1 && r.first_x == b.w - 1 && r.first_y == b.h - 1);
		free(b.p);
	}
}

static void test_rb_swap(void)
{
	/* one pixel with R and B exchanged, XRGB8888 (bytes B, G, R, X) */
	struct buf b = buf_new(640, 480, 0, PATTERN_XRGB8888);
	struct pattern_report r;
	uint32_t x = 100, y = 50;
	uint8_t *p;

	pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
	p = at(&b, x, y);
	CHECK(p[0] != p[2]); /* the test pixel must show the swap */
	uint8_t t = p[0];

	p[0] = p[2];
	p[2] = t;
	CHECK(chk(&b, b.pitch, 1, &r) == 1 && r.first_x == x && r.first_y == y);
	free(b.p);

	/* a whole buffer written as XBGR8888 and read as XRGB8888 */
	for (unsigned s = 0; s < 2; s++) {
		enum pattern_fmt wf = s ? PATTERN_XRGB8888 : PATTERN_XBGR8888;
		enum pattern_fmt rf = s ? PATTERN_XBGR8888 : PATTERN_XRGB8888;
		struct buf w = buf_new(640, 480, 0, wf);
		uint32_t fx = 0, fy = 0;

		pattern_fill(w.p, w.w, w.h, w.pitch, w.bpp, wf);
		/* the first pixel the format's pattern makes differ, found independently */
		for (uint32_t i = 0; i < w.w * w.h; i++) {
			uint32_t px = i % w.w, py = i / w.w;

			if (pattern_pixel(px, py, w.w, wf) != pattern_pixel(px, py, w.w, rf)) {
				fx = px;
				fy = py;
				break;
			}
		}
		CHECK(pattern_check(w.p, w.w, w.h, w.pitch, w.bpp, rf, 1, &r) > 0);
		CHECK(r.first_x == fx && r.first_y == fy);
		CHECK(r.mismatches > w.w * w.h / 2u);
		free(w.p);
	}
}

static void test_pitch(void)
{
	for (unsigned f = 0; f < 3; f++) {
		struct buf b = buf_new(640, 480, 128, fmts[f]);
		struct pattern_report r;
		uint32_t bp = b.bpp / 8u;

		pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
		/* read with a pitch one pixel too long: row 0 still matches, row 1 does not */
		CHECK(chk(&b, b.pitch + bp, 1, &r) > 0);
		CHECK(r.first_y == 1 && r.first_x < b.w);
		CHECK(r.mismatches > b.w * (b.h - 2u) / 2u);
		/* a pitch that is the unpadded width when the buffer is padded */
		CHECK(chk(&b, b.w * bp, 1, &r) > 0 && r.first_y == 1);
		free(b.p);
	}
	/* a pitch below width * bytes is refused, not read out of bounds */
	struct buf b = buf_new(64, 8, 0, PATTERN_XRGB8888);
	struct pattern_report r;

	CHECK(chk(&b, 64 * 4 - 4, 1, &r) == -1);
	free(b.p);
}

static void test_row(void)
{
	for (unsigned f = 0; f < 3; f++) {
		struct buf b = buf_new(640, 480, 64, fmts[f]);
		struct pattern_report r;
		uint32_t y0 = 123, fx = 0;

		pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
		/* row y0 holds row y0 + 1's pixels (a scanout one row off) */
		memcpy(at(&b, 0, y0), at(&b, 0, y0 + 1), b.w * (b.bpp / 8u));
		while (pattern_pixel(fx, y0, b.w, b.fmt) == pattern_pixel(fx, y0 + 1, b.w, b.fmt)) {
			fx++;
		}
		CHECK(chk(&b, b.pitch, 1, &r) > 0);
		CHECK(r.first_y == y0 && r.first_x == fx);
		CHECK(r.expected == pattern_pixel(fx, y0, b.w, b.fmt));
		CHECK(r.got == pattern_pixel(fx, y0 + 1, b.w, b.fmt));
		free(b.p);

		/* the whole image one row down: row 0 is where it first shows */
		b = buf_new(640, 480, 0, fmts[f]);
		pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
		memmove(b.p + b.pitch, b.p, (size_t)b.pitch * (b.h - 1u));
		CHECK(chk(&b, b.pitch, 1, &r) > 0 && r.first_y == 1 && r.first_x == 0);
		free(b.p);
	}
}

static void test_sparse(void)
{
	struct buf b = buf_new(1920, 1080, 256, PATTERN_XRGB8888);
	struct pattern_report r, rf;

	pattern_fill(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt);
	CHECK(chk(&b, b.pitch, 16, &r) == 0);
	CHECK(r.checked == (1920u / 16u) * ((1080u + 15u) / 16u));
	CHECK(chk(&b, b.pitch, 1, &rf) == 0 && rf.checksum != r.checksum);
	CHECK(chk(&b, b.pitch, 0, &rf) == 0 && rf.checked == 1920u * 1080u); /* 0 means 1 */

	at(&b, 160, 320)[1] ^= 4; /* on the grid */
	CHECK(chk(&b, b.pitch, 16, &r) == 1 && r.first_x == 160 && r.first_y == 320);
	at(&b, 160, 320)[1] ^= 4;
	at(&b, 161, 321)[1] ^= 4; /* off the grid: the sparse check misses it, the full one not */
	CHECK(chk(&b, b.pitch, 16, &r) == 0);
	CHECK(chk(&b, b.pitch, 1, &r) == 1 && r.first_x == 161 && r.first_y == 321);
	free(b.p);
}

static void test_layout(void)
{
	static const uint32_t dims[][2] = { { 1920, 1080 }, { 640, 480 }, { 800, 600 },
					    { 37, 21 }, { 64, 8 }, { 1, 1 } };

	for (unsigned i = 0; i < sizeof(dims) / sizeof(dims[0]); i++) {
		uint32_t w = dims[i][0], h = dims[i][1];
		struct pattern_layout l = split_layout(w, h);
		uint8_t *cov = calloc((size_t)w * h, 1);
		const struct pattern_rect *rs[2] = { &l.text, &l.pattern };
		int ok = 1;

		for (int k = 0; k < 2; k++) {
			const struct pattern_rect *r = rs[k];

			ok &= r->x + r->w <= w && r->y + r->h <= h;
			for (uint32_t y = r->y; y < r->y + r->h && y < h; y++) {
				for (uint32_t x = r->x; x < r->x + r->w && x < w; x++) {
					cov[(size_t)y * w + x]++;
				}
			}
		}
		for (size_t j = 0; j < (size_t)w * h; j++) {
			ok &= cov[j] == 1;
		}
		CHECK(ok);
		CHECK(l.text.y == 0 && l.pattern.y == l.text.h);
		CHECK(l.text.h % 16u == 0);
		free(cov);
	}
	/* the Pi 4's proportions: the console on the top half */
	CHECK(split_layout(1920, 1080).text.h == 528 && split_layout(1920, 1080).pattern.h == 552);
	CHECK(split_layout(640, 480).text.h == 240 && split_layout(640, 480).pattern.y == 240);
}

static void test_bands(void)
{
	for (unsigned f = 0; f < 3; f++) {
		struct buf b = buf_new(640, 480, 64, fmts[f]);
		struct buf c = buf_new(640, 480, 64, fmts[f]);
		struct pattern_layout l = split_layout(640, 480);
		struct pattern_report r;
		size_t sz = (size_t)b.pitch * b.h;

		/* the pattern band alone: the text band stays as it was */
		memset(b.p, 0xA5, sz);
		CHECK(pattern_fill_rect(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt, l.pattern) == 0);
		CHECK(b.p[0] == 0xA5 && b.p[(size_t)(l.text.h - 1) * b.pitch + 5] == 0xA5);
		CHECK(pattern_check_rect(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt, l.pattern, 1, &r) == 0);
		CHECK(r.checked == l.pattern.w * l.pattern.h);
		CHECK(pattern_check_rect(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt, l.text, 1, &r) > 0);
		CHECK(r.first_x == 0 && r.first_y == 0);
		/* both bands filled alone give the full fill */
		pattern_fill_rect(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt, l.text);
		pattern_fill(c.p, c.w, c.h, c.pitch, c.bpp, c.fmt);
		CHECK(chk(&b, b.pitch, 1, &r) == 0);
		for (uint32_t y = 0; y < b.h; y++) {
			CHECK(memcmp(at(&b, 0, y), at(&c, 0, y), b.w * (b.bpp / 8u)) == 0);
		}
		/* a rectangle past the screen is clipped, not written */
		struct pattern_rect big = { 600, 470, 5000, 5000 };

		CHECK(pattern_fill_rect(b.p, b.w, b.h, b.pitch, b.bpp, b.fmt, big) == 0);
		free(b.p);
		free(c.p);
	}
}

int main(void)
{
	test_clean();
	test_args();
	test_pixels();
	test_byte_flip();
	test_rb_swap();
	test_pitch();
	test_row();
	test_sparse();
	test_layout();
	test_bands();
	printf("pattern_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
