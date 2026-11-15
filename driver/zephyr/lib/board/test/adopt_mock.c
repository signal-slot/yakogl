/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of driver/zephyr/lib/board/rpi5_scanout/adopt.h: every verdict
 * and reason, and the boundaries of the size, DRAM and reserved checks.
 * Built by tools/v3d/check71.d/adopt.sh.
 */
#include <stdio.h>
#include <string.h>

#include "../rpi5_scanout/adopt.h"

static int total, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		total++;                                                      \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

#define DRAM_BASE 0x0ull
#define DRAM_SIZE 0x80000000ull /* 2 GiB */
#define IMG_BASE 0x00200000ull
#define IMG_SIZE 0x00400000ull
#define FB 0x3e000000ull

/* The mock translation: identity, except 0xdead0000 (not translatable) and
 * 0x10_0000_0000 + x (a bus alias of x). */
static int xlate(void *ctx, uint64_t bus, uint64_t *arm)
{
	(*(int *)ctx)++;
	if (bus == 0xdead0000ull)
		return 0;
	*arm = bus >= 0x1000000000ull ? bus - 0x1000000000ull : bus;
	return 1;
}

static int calls;

static struct rpi5_adopt_env env(void)
{
	struct rpi5_adopt_env e = { DRAM_BASE, DRAM_SIZE, IMG_BASE, IMG_SIZE, xlate, &calls };
	return e;
}

static struct rpi5_adopt_plane plane(void)
{
	struct rpi5_adopt_plane p = { FB, RPI5_ADOPT_FMT_XRGB8888, 7680, 1920, 1080 };
	return p;
}

static int is(struct rpi5_adopt_result r, enum rpi5_adopt_verdict v, const char *why)
{
	return r.verdict == v && r.reason && strcmp(r.reason, why) == 0;
}

int main(void)
{
	struct rpi5_adopt_env e = env();
	struct rpi5_adopt_plane p = plane();
	struct rpi5_adopt_out o, n;
	struct rpi5_adopt_result r;
	uint64_t sz = 7680ull * 1080;

	/* ADOPT, XRGB8888 1080p */
	memset(&o, 0xa5, sizeof o);
	r = rpi5_adopt_check(&p, &e, &o);
	CHECK(is(r, RPI5_ADOPT, "adopt"));
	CHECK(o.phys == FB && o.width == 1920 && o.height == 1080 && o.pitch == 7680 && o.rows == 1080);
	CHECK(o.size == sz && o.bpp == 4 && o.order == RPI5_ADOPT_ORDER_BGR);
	CHECK(rpi5_adopt_scanout_fmt(o.order) == 1);

	/* the other formats and their pixel order */
	p.format = RPI5_ADOPT_FMT_ARGB8888;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT && o.order == RPI5_ADOPT_ORDER_BGR);
	p.format = RPI5_ADOPT_FMT_XBGR8888;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT && o.order == RPI5_ADOPT_ORDER_RGB);
	CHECK(rpi5_adopt_scanout_fmt(o.order) == 0);
	p.format = RPI5_ADOPT_FMT_ABGR8888;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT && o.order == RPI5_ADOPT_ORDER_RGB);
	p.format = RPI5_ADOPT_FMT_RGB565;
	p.pitch = 3840;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT);
	CHECK(o.bpp == 2 && o.order == RPI5_ADOPT_ORDER_NONE && o.size == 3840ull * 1080);
	CHECK(rpi5_adopt_scanout_fmt(o.order) == UINT32_MAX);

	/* the bus alias is translated by the caller's function */
	p = plane();
	p.bus_addr = 0x1000000000ull + FB;
	calls = 0;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT && o.phys == FB && calls == 1);

	/* unknown format */
	p = plane();
	p.format = RPI5_ADOPT_FMT_UNKNOWN;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "unknown format"));
	p.format = (enum rpi5_adopt_format)99;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "unknown format"));

	/* zero size */
	p = plane();
	p.w = 0;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "zero size"));
	p = plane();
	p.h = 0;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "zero size"));

	/* pitch: exactly w*bpp adopts, one below rejects */
	p = plane();
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT);
	p.pitch = 7676;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "pitch below w*bpp"));
	p.pitch = 0;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "pitch below w*bpp"));
	p = plane();
	p.format = RPI5_ADOPT_FMT_RGB565;
	p.pitch = 3839;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "pitch below w*bpp"));
	p = plane();
	p.pitch = 7682;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "pitch not whole pixels"));
	p.pitch = 7684;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT && o.size == 7684ull * 1080);

	/* no address, not translatable */
	p = plane();
	p.bus_addr = 0;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "no address"));
	p.bus_addr = 0xdead0000ull;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "bus address not translatable"));

	/* DRAM: the last byte in, one byte over, past the end, wrapping */
	e.reserved_size = 0;
	p = plane();
	p.bus_addr = DRAM_SIZE - sz;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT);
	p.bus_addr++;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "buffer outside DRAM"));
	p.bus_addr = DRAM_SIZE;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "buffer outside DRAM"));
	p.bus_addr = UINT64_MAX - 100;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "buffer outside DRAM"));
	e.dram_base = 0x40000000ull;
	e.dram_size = 0x40000000ull;
	p.bus_addr = 0x3fffffffull;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "buffer outside DRAM"));
	p.bus_addr = 0x40000000ull;
	CHECK(rpi5_adopt_check(&p, &e, &o).verdict == RPI5_ADOPT);

	/* the reserved range: touching is not overlap */
	e = env();
	p = plane();
	p.bus_addr = IMG_BASE - sz; /* ends at the image's first byte */
	e.dram_base = 0;
	{
		/* a small plane so it fits below the image */
		struct rpi5_adopt_plane s = { 0, RPI5_ADOPT_FMT_XRGB8888, 256, 64, 64 };
		uint64_t ssz = 256ull * 64;
		uint64_t rend = IMG_BASE + IMG_SIZE;

		s.bus_addr = IMG_BASE - ssz;
		CHECK(rpi5_adopt_check(&s, &e, &o).verdict == RPI5_ADOPT);
		s.bus_addr = IMG_BASE - ssz + 1;
		CHECK(is(rpi5_adopt_check(&s, &e, &o), RPI5_ADOPT_REJECT, "buffer overlaps the image"));
		s.bus_addr = rend;
		CHECK(rpi5_adopt_check(&s, &e, &o).verdict == RPI5_ADOPT);
		s.bus_addr = rend - 1;
		CHECK(is(rpi5_adopt_check(&s, &e, &o), RPI5_ADOPT_REJECT, "buffer overlaps the image"));
		s.bus_addr = IMG_BASE + 4096; /* inside the image */
		CHECK(is(rpi5_adopt_check(&s, &e, &o), RPI5_ADOPT_REJECT, "buffer overlaps the image"));
		e.reserved_size = 0; /* no reserved range */
		CHECK(rpi5_adopt_check(&s, &e, &o).verdict == RPI5_ADOPT);
	}
	/* the image inside the buffer */
	e = env();
	p.bus_addr = IMG_BASE - 4096;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "buffer overlaps the image"));

	/* bad arguments; out untouched on a reject */
	p = plane();
	CHECK(is(rpi5_adopt_check(NULL, &e, &o), RPI5_ADOPT_REJECT, "bad argument"));
	CHECK(is(rpi5_adopt_check(&p, NULL, &o), RPI5_ADOPT_REJECT, "bad argument"));
	CHECK(is(rpi5_adopt_check(&p, &e, NULL), RPI5_ADOPT_REJECT, "bad argument"));
	e.bus_to_arm = NULL;
	CHECK(is(rpi5_adopt_check(&p, &e, &o), RPI5_ADOPT_REJECT, "bad argument"));
	e = env();
	memset(&o, 0x5a, sizeof o);
	memcpy(&n, &o, sizeof o);
	p.w = 0;
	(void)rpi5_adopt_check(&p, &e, &o);
	CHECK(memcmp(&o, &n, sizeof o) == 0);

	printf("adopt_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
