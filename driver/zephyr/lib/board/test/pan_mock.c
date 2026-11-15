/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pan_mock: pan_pointer rows, bpp and refusals, pan_write vblank gating.
 * Built by tools/v3d/check71.d/pan.sh.
 */
#include <stdio.h>

#include "pan.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

struct ctx {
	bool vblank;
	int writes;
	uint32_t idx, val;
};

static void wr(void *c, uint32_t i, uint32_t v)
{
	struct ctx *x = c;

	x->writes++;
	x->idx = i;
	x->val = v;
}

static bool vb(void *c)
{
	return ((struct ctx *)c)->vblank;
}

int main(void)
{
	uint32_t p = 0xdead;
	const uint64_t base = 0x10000000;
	struct ctx c = { 0 };

	/* 1280x1440 surface at 4 bpp, 720-line view */
	CHECK(pan_pointer(base, 5120, 0, 4, 0, 1440, 720, &p) == PAN_OK && p == base);
	CHECK(pan_pointer(base, 5120, 1, 4, 0, 1440, 720, &p) == PAN_OK && p == base + 5120);
	CHECK(pan_pointer(base, 5120, 720, 4, 0, 1440, 720, &p) == PAN_OK &&
	      p == base + 720ull * 5120);
	CHECK(pan_pointer(base, 5120, 721, 4, 0, 1440, 720, &p) == PAN_ERR_ROW);
	CHECK(pan_pointer(base, 5120, 1440, 4, 0, 1440, 720, &p) == PAN_ERR_ROW);
	CHECK(pan_pointer(base, 5120, 0xffffffffu, 4, 0, 1440, 720, &p) == PAN_ERR_ROW);
	/* column offset */
	CHECK(pan_pointer(base, 5120, 2, 4, 4, 1440, 720, &p) == PAN_OK &&
	      p == base + 2 * 5120 + 16);
	CHECK(pan_pointer(base, 5120, 2, 4, 1, 1440, 720, &p) == PAN_ERR_ALIGN);
	CHECK(pan_pointer(base, 5120, 0, 4, 1280, 1440, 720, &p) == PAN_ERR_ARG);
	/* 2 bpp */
	CHECK(pan_pointer(base, 2560, 3, 2, 0, 1440, 720, &p) == PAN_OK && p == base + 3 * 2560);
	CHECK(pan_pointer(base, 2560, 3, 2, 8, 1440, 720, &p) == PAN_OK &&
	      p == base + 3 * 2560 + 16);
	CHECK(pan_pointer(base, 2560, 3, 2, 3, 1440, 720, &p) == PAN_ERR_ALIGN);
	/* misaligned pitch, base, bad args, range */
	CHECK(pan_pointer(base, 5124, 0, 4, 0, 1440, 720, &p) == PAN_ERR_ALIGN);
	CHECK(pan_pointer(base + 4, 5120, 0, 4, 0, 1440, 720, &p) == PAN_ERR_ALIGN);
	CHECK(pan_pointer(base, 0, 0, 4, 0, 1440, 720, &p) == PAN_ERR_ARG);
	CHECK(pan_pointer(base, 5120, 0, 3, 0, 1440, 720, &p) == PAN_ERR_ARG);
	CHECK(pan_pointer(base, 5120, 0, 4, 0, 100, 720, &p) == PAN_ERR_ARG);
	CHECK(pan_pointer(base, 5120, 0, 4, 0, 1440, 0, &p) == PAN_ERR_ARG);
	CHECK(pan_pointer(base, 5120, 0, 4, 0, 1440, 720, NULL) == PAN_ERR_ARG);
	CHECK(pan_pointer(0xfffff000ull, 5120, 720, 4, 0, 1440, 720, &p) == PAN_ERR_RANGE);
	/* a refused call leaves *out alone */
	p = 0x1234;
	CHECK(pan_pointer(base, 5120, 999, 4, 0, 1440, 720, &p) == PAN_ERR_ROW && p == 0x1234);
	/* single-line-fit view: row 0 is also the last row */
	CHECK(pan_pointer(base, 5120, 0, 4, 0, 720, 720, &p) == PAN_OK);
	CHECK(pan_pointer(base, 5120, 1, 4, 0, 720, 720, &p) == PAN_ERR_ROW);

	/* pan_write */
	c.vblank = false;
	CHECK(pan_write(wr, vb, &c, 5, 0xabc0) == PAN_ERR_VBLANK && c.writes == 0);
	c.vblank = true;
	CHECK(pan_write(wr, vb, &c, 5, 0xabc0) == PAN_OK && c.writes == 1 && c.idx == 5 &&
	      c.val == 0xabc0);
	CHECK(pan_write(NULL, vb, &c, 5, 1) == PAN_ERR_WRITER && c.writes == 1);
	CHECK(pan_write(wr, NULL, &c, 5, 1) == PAN_ERR_WRITER && c.writes == 1);

	printf("pan_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
