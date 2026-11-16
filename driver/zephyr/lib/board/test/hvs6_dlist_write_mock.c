/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_dlist_write_mock: every list hvs6w_write_plane writes is decoded again
 * with hvs6_dlist_first_plane and the fields compared; the error paths are
 * checked and leave the buffer alone. Built by
 * tools/v3d/check71.d/hvs6_dlist_write.sh.
 */
#include <stdio.h>
#include <string.h>

#include "hvs6_dlist_write.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

static void roundtrip(const struct hvs6w_plane *p, uint32_t exp_code, uint32_t exp_order,
		      uint32_t start)
{
	uint32_t buf[64 + HVS6W_LIST_WORDS];
	struct hvs6_plane d;
	int n;

	memset(buf, 0xa5, sizeof(buf));
	n = hvs6w_write_plane(buf + start, HVS6W_LIST_WORDS, p);
	CHECK(n == (int)HVS6W_LIST_WORDS);
	CHECK(hvs6_dlist_first_plane(buf, start + HVS6W_LIST_WORDS, start, &d) == HVS6_OK);
	CHECK(d.addr == p->addr);
	CHECK(d.pitch == p->pitch);
	CHECK(d.width == p->width && d.height == p->height);
	CHECK(d.x == p->x && d.y == p->y);
	CHECK(d.format_code == exp_code);
	CHECK(d.order == exp_order);
	CHECK(d.rgb32 == (exp_code == HVS6_FMT_RGBA8888));
	CHECK(d.unity && d.nwords == HVS6W_ENTRY_WORDS);
	/* the END word follows the entry, and an END at the start decodes as empty */
	CHECK(buf[start + HVS6W_ENTRY_WORDS] == HVS6_CTL0_END);
	CHECK(hvs6_dlist_first_plane(buf, start + HVS6W_LIST_WORDS, start + HVS6W_ENTRY_WORDS,
				     &d) == HVS6_ERR_END);
	/* nothing outside the list is written */
	CHECK(start == 0 || buf[start - 1] == 0xa5a5a5a5u);
	CHECK(buf[start + HVS6W_LIST_WORDS] == 0xa5a5a5a5u);
	/* the entry has no tiling and no flip bits */
	CHECK(((buf[start] >> HVS6_CTL0_ADDR_MODE_SHIFT) & HVS6_CTL0_ADDR_MODE_MASK) ==
	      HVS6_CTL0_ADDR_MODE_LINEAR);
	CHECK(!(buf[start + 5] & HVS6_PTR0_VFLIP));
}

static void errcase(int want, struct hvs6w_plane p, size_t room)
{
	uint32_t buf[16];
	uint32_t ref[16];

	memset(buf, 0x5a, sizeof(buf));
	memcpy(ref, buf, sizeof(buf));
	CHECK(hvs6w_write_plane(buf, room, &p) == want);
	CHECK(memcmp(buf, ref, sizeof(buf)) == 0);
}

int main(void)
{
	static const struct hvs6w_plane base = {HVS6W_XRGB8888, 0, 0, 1920, 1080, 7680, 0x3b000000};
	struct hvs6w_plane p;
	uint32_t buf[16];

	roundtrip(&base, HVS6_FMT_RGBA8888, HVS6_ORDER_ARGB, 0);
	p = base;
	p.format = HVS6W_ARGB8888;
	roundtrip(&p, HVS6_FMT_RGBA8888, HVS6_ORDER_ARGB, 32);
	p = (struct hvs6w_plane){HVS6W_RGB565, 100, 200, 640, 240, 1300 * 2 & ~3u, 0x1234567890ull};
	roundtrip(&p, HVS6_FMT_RGB565, HVS6_ORDER_RGBA, 32);
	/* the limits: smallest, largest, 40-bit address */
	p = (struct hvs6w_plane){HVS6W_RGB565, 0, 0, 1, 1, 4, 0};
	roundtrip(&p, HVS6_FMT_RGB565, HVS6_ORDER_RGBA, 0);
	p = (struct hvs6w_plane){HVS6W_XRGB8888, 0x1fff, 0x1fff, 8192, 8192, 0x1fffc,
				 0xffffffffffull};
	roundtrip(&p, HVS6_FMT_RGBA8888, HVS6_ORDER_ARGB, 5);
	/* the words, once, against the cited positions */
	p = (struct hvs6w_plane){HVS6W_XRGB8888, 3, 5, 16, 8, 64, 0x2aabcd1000ull};
	CHECK(hvs6w_write_plane(buf, 16, &p) == 9);
	CHECK(buf[0] == (0x40000000u | (8u << 24) | 0x8000u | (2u << 13) | 7u));
	CHECK(buf[1] == ((5u << 16) | 3u));
	CHECK(buf[3] == ((7u << 16) | 15u));
	CHECK(buf[5] == 0x2au && buf[6] == 0xabcd1000u && buf[7] == 64u);
	CHECK(buf[8] == 0x80000000u);

	/* errors */
	errcase(HVS6W_ERR_BUFFER, base, 8);
	errcase(HVS6W_ERR_BUFFER, base, 0);
	CHECK(hvs6w_write_plane(NULL, 16, &base) == HVS6W_ERR_BUFFER);
	CHECK(hvs6w_write_plane(buf, 16, NULL) == HVS6W_ERR_BUFFER);
	p = base; p.format = 0;  errcase(HVS6W_ERR_FORMAT, p, 16);
	p = base; p.format = 99; errcase(HVS6W_ERR_FORMAT, p, 16);
	p = base; p.width = 0;    errcase(HVS6W_ERR_SIZE, p, 16);
	p = base; p.height = 0;   errcase(HVS6W_ERR_SIZE, p, 16);
	p = base; p.width = 8193; p.pitch = 0x10000; errcase(HVS6W_ERR_SIZE, p, 16);
	p = base; p.height = 8193; errcase(HVS6W_ERR_SIZE, p, 16);
	p = base; p.x = 0x2000;   errcase(HVS6W_ERR_POS, p, 16);
	p = base; p.y = 0x2000;   errcase(HVS6W_ERR_POS, p, 16);
	p = base; p.pitch = 7676; errcase(HVS6W_ERR_PITCH, p, 16);  /* under a row */
	p = base; p.pitch = 7682; errcase(HVS6W_ERR_PITCH, p, 16);  /* unaligned */
	p = base; p.width = 8192; p.pitch = 0x20000; errcase(HVS6W_ERR_PITCH, p, 16);
	p = base; p.addr = 1ull << 40; errcase(HVS6W_ERR_ADDR, p, 16);

	if (failed) {
		printf("hvs6_dlist_write_mock: %d of %d checks, %d failed\n", checks - failed,
		       checks, failed);
		return 1;
	}
	printf("hvs6_dlist_write_mock: %d of %d checks, 0 failed\n", checks, checks);
	return 0;
}
