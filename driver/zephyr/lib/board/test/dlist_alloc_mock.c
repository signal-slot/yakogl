/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dlist_alloc_mock: placement and swap of display lists around the firmware's
 * live spans. Built by tools/v3d/check71.d/dlist_alloc.sh.
 */
#include <stdio.h>

#include "dlist_alloc.h"

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

#define MEM 1024u

static int overlap(uint32_t a, uint32_t al, uint32_t b, uint32_t bl)
{
	return a < b + bl && b < a + al;
}

/* the invariants every successful placement keeps */
static void valid(const struct dla_slots *s, uint32_t mem, uint32_t align,
		  const struct dla_span *sp, size_t n)
{
	CHECK(s->start[0] >= DLA_BOOTLOADER_END && s->start[1] >= DLA_BOOTLOADER_END);
	CHECK(s->start[0] % align == 0 && s->start[1] % align == 0);
	CHECK(s->start[0] + s->len <= mem && s->start[1] + s->len <= mem);
	CHECK(!overlap(s->start[0], s->len, s->start[1], s->len));
	for (size_t i = 0; i < n; i++) {
		CHECK(!overlap(s->start[0], s->len, sp[i].start, sp[i].len));
		CHECK(!overlap(s->start[1], s->len, sp[i].start, sp[i].len));
	}
}

int main(void)
{
	struct dla_slots s;
	uint32_t p;

	/* empty memory: right after the bootloader's words */
	CHECK(dlist_place(MEM, 16, 1, NULL, 0, &s) == DLA_OK);
	CHECK(s.start[0] == 32 && s.start[1] == 48 && s.len == 16 && s.front == 0);
	valid(&s, MEM, 1, NULL, 0);

	/* firmware list at the start */
	{
		struct dla_span sp[] = {{0, 64}};

		CHECK(dlist_place(MEM, 16, 1, sp, 1, &s) == DLA_OK);
		CHECK(s.start[0] == 64 && s.start[1] == 80);
		valid(&s, MEM, 1, sp, 1);
	}
	/* firmware list at the end */
	{
		struct dla_span sp[] = {{MEM - 100, 100}};

		CHECK(dlist_place(MEM, 16, 1, sp, 1, &s) == DLA_OK);
		CHECK(s.start[0] == 32 && s.start[1] == 48);
		valid(&s, MEM, 1, sp, 1);
	}
	/* in the middle, directly after the kept words: the slots jump over it */
	{
		struct dla_span sp[] = {{40, 20}};

		CHECK(dlist_place(MEM, 16, 1, sp, 1, &s) == DLA_OK);
		CHECK(s.start[0] == 60 && s.start[1] == 76);
		valid(&s, MEM, 1, sp, 1);
	}
	/* a hole between spans (given unsorted) is used only when big enough */
	{
		struct dla_span sp[] = {{100, 50}, {32, 60}, {170, 30}};

		CHECK(dlist_place(MEM, 16, 1, sp, 3, &s) == DLA_OK);
		CHECK(s.start[0] == 150 && s.start[1] == 200);
		valid(&s, MEM, 1, sp, 3);
		CHECK(dlist_place(MEM, 20, 1, sp, 3, &s) == DLA_OK);
		CHECK(s.start[0] == 150 && s.start[1] == 200);
		CHECK(dlist_place(MEM, 21, 1, sp, 3, &s) == DLA_OK);
		CHECK(s.start[0] == 200 && s.start[1] == 221);
		valid(&s, MEM, 1, sp, 3);
	}
	/* no room: one slot fits, two do not; the exact fit passes */
	{
		struct dla_span sp[] = {{32, MEM - 32 - 16}};
		struct dla_slots keep = {{7, 8}, 9, 1};

		s = keep;
		CHECK(dlist_place(MEM, 16, 1, sp, 1, &s) == DLA_ERR_NOROOM);
		CHECK(s.start[0] == 7 && s.start[1] == 8 && s.len == 9 && s.front == 1);
		CHECK(dlist_place(MEM, MEM - 32 + 1, 1, NULL, 0, &s) == DLA_ERR_NOROOM);
		CHECK(dlist_place(MEM, (MEM - 32) / 2, 1, NULL, 0, &s) == DLA_OK);
		CHECK(s.start[0] == 32 && s.start[1] == 32 + (MEM - 32) / 2);
		CHECK(dlist_place(MEM, (MEM - 32) / 2 + 1, 1, NULL, 0, &s) == DLA_ERR_NOROOM);
	}
	/* alignment */
	{
		struct dla_span sp[] = {{32, 5}};

		CHECK(dlist_place(MEM, 10, 16, sp, 1, &s) == DLA_OK);
		CHECK(s.start[0] == 48 && s.start[1] == 64);
		valid(&s, MEM, 16, sp, 1);
		CHECK(dlist_place(MEM, 10, 64, NULL, 0, &s) == DLA_OK);
		CHECK(s.start[0] == 64 && s.start[1] == 128);
		valid(&s, MEM, 64, NULL, 0);
		/* alignment can cost the last slot */
		CHECK(dlist_place(100, 10, 64, NULL, 0, &s) == DLA_ERR_NOROOM);
	}
	/* refusals */
	CHECK(dlist_place(MEM, 0, 1, NULL, 0, &s) == DLA_ERR_ARG);
	CHECK(dlist_place(MEM, 8, 0, NULL, 0, &s) == DLA_ERR_ARG);
	CHECK(dlist_place(MEM, 8, 3, NULL, 0, &s) == DLA_ERR_ARG);
	CHECK(dlist_place(MEM, 8, 1, NULL, 1, &s) == DLA_ERR_ARG);
	CHECK(dlist_place(MEM, 8, 1, NULL, 0, NULL) == DLA_ERR_ARG);
	{
		struct dla_span bad[] = {{MEM - 4, 8}};
		struct dla_span wrap[] = {{0xfffffff0u, 0x20}};
		struct dla_span many[DLA_MAX_LIVE + 1] = {{0, 0}};

		CHECK(dlist_place(MEM, 8, 1, bad, 1, &s) == DLA_ERR_SPAN);
		CHECK(dlist_place(MEM, 8, 1, wrap, 1, &s) == DLA_ERR_SPAN);
		CHECK(dlist_place(MEM, 8, 1, many, DLA_MAX_LIVE + 1, &s) == DLA_ERR_ARG);
		CHECK(dlist_place(MEM, 8, 1, many, DLA_MAX_LIVE, &s) == DLA_OK);
	}
	/* an empty span blocks nothing; memory smaller than the kept words */
	{
		struct dla_span sp[] = {{40, 0}};

		CHECK(dlist_place(MEM, 16, 1, sp, 1, &s) == DLA_OK && s.start[0] == 32);
		CHECK(dlist_place(16, 4, 1, NULL, 0, &s) == DLA_ERR_NOROOM);
	}

	/* repeated swaps alternate and never hand out the slot being scanned */
	CHECK(dlist_place(MEM, 16, 1, NULL, 0, &s) == DLA_OK);
	CHECK(dlist_back(&s) == 1);
	for (int i = 0; i < 6; i++) {
		int scanned_before = s.front;
		int back = dlist_back(&s);
		int next = dlist_swap(&s, &p);

		CHECK(back != scanned_before);
		CHECK(p == s.start[back] && s.front == back);
		CHECK(next == scanned_before && next == dlist_back(&s));
		CHECK(p == (i % 2 == 0 ? 48u : 32u));
	}
	CHECK(dlist_swap(NULL, &p) == -1 && dlist_swap(&s, NULL) == -1);
	s.front = 2;
	CHECK(dlist_swap(&s, &p) == -1);

	printf("dlist_alloc_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
