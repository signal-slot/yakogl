/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fb_flush_mock: fb_flush.h against a recording clean callback. Built by
 * tools/v3d/check71.d/fb_flush.sh.
 */
#include <stdio.h>
#include <string.h>

#include "fb_flush.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

struct rec {
	int n;
	uint64_t addr[16], len[16];
};

static void rec_fn(uint64_t addr, uint64_t len, void *ctx)
{
	struct rec *r = ctx;

	if (r->n < 16) {
		r->addr[r->n] = addr;
		r->len[r->n] = len;
	}
	r->n++;
}

static uint64_t ctr_a76(void)
{
	return 0x8444c004ull; /* DminLine = 4: 64 bytes */
}

static uint64_t ctr_32(void)
{
	return 3ull << 16;
}

static void test_range(void)
{
	struct fb_range r;

	r = fb_flush_range(0x1000, 64, 64);
	CHECK(r.start == 0x1000 && r.end == 0x1040);
	r = fb_flush_range(0x1001, 64, 64); /* unaligned base crosses a line */
	CHECK(r.start == 0x1000 && r.end == 0x1080);
	r = fb_flush_range(0x103f, 1, 64);
	CHECK(r.start == 0x1000 && r.end == 0x1040);
	r = fb_flush_range(0x1040, 1, 64);
	CHECK(r.start == 0x1040 && r.end == 0x1080);
	r = fb_flush_range(0x1000, 65, 64);
	CHECK(r.start == 0x1000 && r.end == 0x1080);
	r = fb_flush_range(0x1010, 0x30, 64); /* ends exactly on a line end */
	CHECK(r.start == 0x1000 && r.end == 0x1040);
	r = fb_flush_range(0x1000, 0, 64);
	CHECK(r.start == r.end);
	r = fb_flush_range(0x1000, 8, 0);
	CHECK(r.start == r.end);
	r = fb_flush_range(0x1000, 8, 48);
	CHECK(r.start == r.end);
	r = fb_flush_range(~0ull - 3u, 16, 64); /* wraps */
	CHECK(r.start == r.end);
	r = fb_flush_range(~0ull - 63u, 64, 64); /* end would be 2^64 */
	CHECK(r.start == r.end);
	r = fb_flush_range(0x40, 8, 4); /* a 4-byte line */
	CHECK(r.start == 0x40 && r.end == 0x48);
}

static void test_line(void)
{
	CHECK(fb_flush_line(NULL) == 64);
	CHECK(fb_flush_line(ctr_a76) == 64);
	CHECK(fb_flush_line(ctr_32) == 32);
}

static void test_rows(void)
{
	struct fb_dirty d;
	struct rec r;

	/* 100 x 10, 4 bytes, pitch 512 > 400 */
	CHECK(fb_dirty_init(&d, 0x10000, 100, 10, 4, 512, 64, 4) == 0);
	CHECK(fb_dirty_mark(&d, 2, 1) == 1);
	memset(&r, 0, sizeof(r));
	CHECK(fb_dirty_flush(&d, rec_fn, &r) == 1);
	/* one row: 400 bytes from 0x10000 + 1024 -> 7 lines */
	CHECK(r.n == 1 && r.addr[0] == 0x10400 && r.len[0] == 448);
	CHECK(fb_dirty_flush(&d, rec_fn, &r) == 0); /* emptied */

	/* two rows: the gap after the first row is inside the range */
	fb_dirty_mark(&d, 2, 2);
	memset(&r, 0, sizeof(r));
	fb_dirty_flush(&d, rec_fn, &r);
	CHECK(r.n == 1 && r.addr[0] == 0x10400 && r.len[0] == 512 + 448);

	/* overlapping and adjacent merge into one range */
	fb_dirty_mark(&d, 1, 3);
	fb_dirty_mark(&d, 2, 4);
	fb_dirty_mark(&d, 6, 1);
	CHECK(d.n == 1 && d.r[0].y0 == 1 && d.r[0].y1 == 7);
	fb_dirty_flush(&d, rec_fn, &r);

	/* clamped: beyond the surface, and zero count */
	CHECK(fb_dirty_mark(&d, 8, 10) == 2);
	CHECK(fb_dirty_mark(&d, 10, 1) == 0);
	CHECK(fb_dirty_mark(&d, 3, 0) == 0);
	CHECK(d.n == 1 && d.r[0].y0 == 8 && d.r[0].y1 == 10);
	fb_dirty_flush(&d, rec_fn, &r);
}

static void test_limit(void)
{
	struct fb_dirty d;
	struct rec r;

	CHECK(fb_dirty_init(&d, 0, 64, 100, 4, 256, 64, 2) == 0);
	fb_dirty_mark(&d, 0, 1);
	fb_dirty_mark(&d, 10, 1);
	fb_dirty_mark(&d, 12, 1); /* gap 1 to the second: merges with it */
	CHECK(d.n == 2);
	CHECK(d.r[0].y0 == 0 && d.r[0].y1 == 1);
	CHECK(d.r[1].y0 == 10 && d.r[1].y1 == 13);
	fb_dirty_mark(&d, 50, 1); /* 3 ranges: the smallest gap (0..1, 10..13 = 9) goes */
	CHECK(d.n == 2 && d.r[0].y0 == 0 && d.r[0].y1 == 13 && d.r[1].y0 == 50);
	memset(&r, 0, sizeof(r));
	CHECK(fb_dirty_flush(&d, rec_fn, &r) == 2);
	CHECK(r.addr[0] == 0 && r.len[0] == 12 * 256 + 256);
	CHECK(r.addr[1] == 50 * 256 && r.len[1] == 256);

	/* one range allowed: everything between is covered */
	CHECK(fb_dirty_init(&d, 0, 64, 100, 4, 256, 64, 1) == 0);
	fb_dirty_mark(&d, 5, 1);
	fb_dirty_mark(&d, 90, 1);
	CHECK(d.n == 1 && d.r[0].y0 == 5 && d.r[0].y1 == 91);

	/* ranges whose aligned bytes touch are passed as one call: pitch 96
	 * puts rows 0 and 1 (width*bpp 64) in lines 0 and 1, row 2 starts at
	 * 192 */
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 96, 64, 8) == 0);
	fb_dirty_mark(&d, 0, 1);
	fb_dirty_mark(&d, 2, 1);
	memset(&r, 0, sizeof(r));
	CHECK(d.n == 2);
	fb_dirty_flush(&d, rec_fn, &r);
	CHECK(r.n == 2); /* row 0 ends at 64, row 2 starts at 192 */
	CHECK(fb_dirty_init(&d, 0, 8, 10, 4, 40, 64, 8) == 0);
	fb_dirty_mark(&d, 0, 1); /* bytes 0..32 -> line 0 */
	fb_dirty_mark(&d, 2, 1); /* bytes 80..112 -> line 1 */
	memset(&r, 0, sizeof(r));
	fb_dirty_flush(&d, rec_fn, &r);
	CHECK(r.n == 1 && r.addr[0] == 0 && r.len[0] == 128); /* touching: one call */
}

static void test_bad_init(void)
{
	struct fb_dirty d;

	CHECK(fb_dirty_init(&d, 0, 0, 10, 4, 64, 64, 4) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 0, 4, 64, 64, 4) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 0, 64, 64, 4) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 63, 64, 4) == -1); /* pitch < width*bpp */
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 64, 64, 0) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 64, 64, FB_DIRTY_MAX + 1u) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 64, 48, 4) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 64, 0, 4) == -1);
	CHECK(fb_dirty_init_area(&d, 0, 16, 10, 4, 64, 5, 5, 64, 4) == -1);
	CHECK(fb_dirty_init_area(&d, 0, 16, 10, 4, 64, 5, 11, 64, 4) == -1);
	CHECK(fb_dirty_init(&d, 0, 16, 10, 4, 64, 64, FB_DIRTY_MAX) == 0);
}

static void test_split(void)
{
	/* 1920x1080x4, text = rows 0..539, pattern = rows 540..1079 */
	struct fb_dirty txt, pat;
	struct rec r;
	const uint64_t base = 0x3e000000ull, pitch = 1920u * 4u;

	CHECK(fb_dirty_init_area(&txt, base, 1920, 1080, 4, 7680, 0, 540, 64, 4) == 0);
	CHECK(fb_dirty_init_area(&pat, base, 1920, 1080, 4, 7680, 540, 1080, 64, 4) == 0);
	CHECK(fb_dirty_mark(&txt, 0, 1080) == 540); /* clamped to the area */
	CHECK(fb_dirty_mark(&pat, 0, 1080) == 540);
	CHECK(pat.r[0].y0 == 540);

	memset(&r, 0, sizeof(r));
	CHECK(fb_dirty_flush(&txt, rec_fn, &r) == 1);
	CHECK(r.addr[0] == base && r.len[0] == 540 * pitch);
	CHECK(txt.n == 0);
	CHECK(pat.n != 0); /* the other area is untouched */

	memset(&r, 0, sizeof(r));
	CHECK(fb_dirty_flush(&pat, rec_fn, &r) == 1);
	CHECK(r.addr[0] == base + 540 * pitch && r.len[0] == 540 * pitch);

	/* the whole 1920x1080x4 surface is 8294400 bytes */
	{
		struct fb_dirty all;

		CHECK(fb_dirty_init(&all, base, 1920, 1080, 4, 7680, 64, 1) == 0);
		fb_dirty_mark(&all, 0, 1080);
		memset(&r, 0, sizeof(r));
		CHECK(fb_dirty_flush(&all, rec_fn, &r) == 1);
		CHECK(r.addr[0] == base && r.len[0] == 8294400u);
	}
	/* an unaligned base shifts the range out by one line */
	{
		struct fb_dirty u;

		CHECK(fb_dirty_init(&u, base + 8, 1920, 1080, 4, 7680, 64, 1) == 0);
		fb_dirty_mark(&u, 1079, 1);
		memset(&r, 0, sizeof(r));
		fb_dirty_flush(&u, rec_fn, &r);
		CHECK(r.addr[0] == base + 1079 * pitch && r.len[0] == pitch + 64);
	}
}

int main(void)
{
	test_range();
	test_line();
	test_rows();
	test_limit();
	test_bad_init();
	test_split();
	printf("fb_flush_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
