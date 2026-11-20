/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host test of ../src/hold_check.h. Prints "hold_check_test: N checks, F failed". */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "../src/hold_check.h"
#include "../../../../../driver/zephyr/lib/board/rpi5_scanout/pattern.h"

static int checks, failed;
#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

#define W 64u
#define H 32u
#define PITCH (W * 2u)

static uint32_t expect_px(uint32_t x, uint32_t y, void *ctx)
{
	(void)ctx;
	return pattern_pixel(x, y, W, PATTERN_RGB565);
}

static void fill(uint8_t *p)
{
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			uint32_t v = expect_px(x, y, NULL);

			p[y * PITCH + x * 2] = v & 0xff;
			p[y * PITCH + x * 2 + 1] = v >> 8;
		}
}

static uint32_t st(uint32_t frcnt)
{
	return (frcnt & 0x3fu) << 16 | 0x2000u;
}

static struct hold_sample good(uint32_t frcnt)
{
	struct hold_sample s = { st(frcnt), 0x20, 0x1111, 0x2222, 0, 0x1, 0x1 };

	return s;
}

static void test_hash(void)
{
	static uint8_t p[PITCH * H];
	uint32_t h0;

	CHECK(hold_fnv1a("", 0) == 0x811c9dc5u);
	CHECK(hold_fnv1a("a", 1) == 0xe40c292cu);
	CHECK(hold_fnv1a("foobar", 6) == 0xbf9cf968u);

	fill(p);
	h0 = hold_rows_hash(p, PITCH, H, 8);
	CHECK(h0 == hold_rows_hash(p, PITCH, H, 8));
	CHECK(hold_row_at(0, 8, H) == 0 && hold_row_at(7, 8, H) == 28);
	p[PITCH * 4 + 3] ^= 1; /* row 4 is one of the 8 (0,4,8,...) */
	CHECK(hold_rows_hash(p, PITCH, H, 8) != h0);
	p[PITCH * 4 + 3] ^= 1;
	p[PITCH * 5] ^= 1; /* row 5 is not sampled */
	CHECK(hold_rows_hash(p, PITCH, H, 8) == h0);
	CHECK(hold_rows_hash(p, PITCH, H, 100) == hold_rows_hash(p, PITCH, H, H));
}

static void test_mismatch(void)
{
	static uint8_t p[PITCH * H];
	struct hold_mismatch m;

	fill(p);
	m = hold_mismatch_scan(p, PITCH, W, H, 2, expect_px, NULL);
	CHECK(m.count == 0 && m.rows == 0);

	p[7 * PITCH + 9 * 2] ^= 0x04; /* single pixel (9, 7) */
	m = hold_mismatch_scan(p, PITCH, W, H, 2, expect_px, NULL);
	CHECK(m.count == 1 && m.first_x == 9 && m.first_y == 7 && m.rows == 1);

	p[3 * PITCH + 60 * 2 + 1] ^= 0x80; /* (60, 3), an earlier row */
	p[3 * PITCH + 2] ^= 0x01;          /* (1, 3), earlier in that row */
	m = hold_mismatch_scan(p, PITCH, W, H, 2, expect_px, NULL);
	CHECK(m.count == 3 && m.first_x == 1 && m.first_y == 3 && m.rows == 2);

	memset(p, 0, sizeof p); /* full-screen change */
	m = hold_mismatch_scan(p, PITCH, W, H, 2, expect_px, NULL);
	CHECK(m.rows == H && m.count > W * H * 9 / 10 && m.first_y == 0);
}

static void test_frcnt(void)
{
	uint32_t a[4] = { st(1), st(2), st(3), st(4) };
	uint32_t wrap[4] = { st(62), st(63), st(0), st(1) };
	uint32_t stall[4] = { st(5), st(6), st(6), st(7) };
	uint32_t low[3] = { st(5) | 0x123u, st(6) | 0x7ffu, st(7) };
	uint32_t laps[2] = { st(9), st(9 + 64) }; /* identical 6-bit value */

	CHECK(hold_frcnt_stall(a, 4) == -1);
	CHECK(hold_frcnt_stall(wrap, 4) == -1);
	CHECK(hold_frcnt_stall(stall, 4) == 2);
	CHECK(hold_frcnt_stall(low, 3) == -1);
	CHECK(hold_frcnt_stall(laps, 2) == 1);
	CHECK(hold_frcnt_stall(a, 1) == -1 && hold_frcnt_stall(a, 0) == -1);
	CHECK(hold_frcnt(0xffffffffu) == 63);
}

static void test_regs(void)
{
	CHECK(hold_unclocked(0xffffffffu) && !hold_unclocked(0xfffffffeu) && !hold_unclocked(0));
	CHECK(hold_pv_enabled(1, 1) && hold_pv_enabled(0x81, 0x3));
	CHECK(!hold_pv_enabled(0, 1) && !hold_pv_enabled(1, 0) && !hold_pv_enabled(0, 0));
}

static void test_clamp(void)
{
	bool c = true;

	CHECK(hold_clamp_end(5000, 20000, 60, &c) == 25000 && !c);
	CHECK(hold_clamp_end(5000, 20000, 30, &c) == 20000 && c); /* shortened */
	CHECK(hold_clamp_end(5000, 20000, 25, &c) == 15000 && c);
	CHECK(hold_clamp_end(5000, 20000, 35, &c) == 25000 && !c); /* equal: not clamped */
	CHECK(hold_clamp_end(5000, 20000, 5, &c) == 5000 && c);    /* deadline too close */
	CHECK(hold_clamp_end(12000, 100, 20, &c) == 12000 && c); /* start past the deadline */
	CHECK(hold_clamp_end(1000, 100, 20, NULL) == 1100);
	CHECK(hold_clamp_end(0, 0, 60, &c) == 0 && !c);
}

static void fresh(struct hold_sample *s, uint32_t first)
{
	for (uint32_t i = 0; i < 4; i++)
		s[i] = good(first + i);
}

static void test_judge(void)
{
	struct hold_sample ref = good(10), s[4];
	struct hold_verdict v;

	fresh(s, 11);
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.pass && v.reason == HOLD_PASS && v.index == -1 && v.redraw == HOLD_REDRAW_NO);

	/* FRCNT wraps 63 -> 0 and still passes */
	ref = good(62);
	fresh(s, 63);
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.pass);

	ref = good(10);
	fresh(s, 11);
	s[2].plane_hash ^= 1;
	s[2].rows_touched = 1; /* single pixel */
	v = hold_judge(&ref, s, 4, false);
	CHECK(!v.pass && v.reason == HOLD_PLANE_CHANGED && v.index == 2 && v.rows_touched == 1);
	CHECK(v.redraw == HOLD_REDRAW_B);

	fresh(s, 11);
	s[0].plane_hash ^= 1;
	s[0].rows_touched = 1080; /* full screen */
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_PLANE_CHANGED && v.index == 0 && v.rows_touched == 1080);

	fresh(s, 11);
	s[1].list_hash ^= 1;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_LIST_CHANGED && v.index == 1 && v.redraw == HOLD_REDRAW_NO);

	fresh(s, 11);
	s[3].dl_ptr = 0x40;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_POINTER_CHANGED && v.index == 3);

	fresh(s, 11);
	s[2] = good(12); /* same FRCNT as s[1] */
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_FRCNT_STALLED && v.index == 2);

	fresh(s, 11);
	s[0] = good(10); /* stalled against the reference */
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_FRCNT_STALLED && v.index == 0);

	fresh(s, 11);
	s[1].pv_vcontrol = 0;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_PV_DISABLED && v.index == 1);
	s[1] = good(12);
	s[1].pv_control = 0;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_PV_DISABLED && v.index == 1);

	fresh(s, 11);
	s[2].status = 0xffffffffu; /* unclocked outranks everything */
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_UNCLOCKED_FAIL && v.index == 2);
	fresh(s, 11);
	s[0].pv_control = 0xffffffffu;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_UNCLOCKED_FAIL && v.index == 0);

	/* the first failing sample wins */
	fresh(s, 11);
	s[1].list_hash ^= 1;
	s[2].dl_ptr = 7;
	v = hold_judge(&ref, s, 4, false);
	CHECK(v.reason == HOLD_LIST_CHANGED && v.index == 1);

	/* redraw phase A, with or without a hold failure */
	fresh(s, 11);
	v = hold_judge(&ref, s, 4, true);
	CHECK(v.pass && v.redraw == HOLD_REDRAW_A);
	s[1].plane_hash ^= 1;
	v = hold_judge(&ref, s, 4, true);
	CHECK(!v.pass && v.redraw == HOLD_REDRAW_A);

	/* an empty hold passes */
	v = hold_judge(&ref, s, 0, false);
	CHECK(v.pass && v.redraw == HOLD_REDRAW_NO);

	CHECK(strcmp(hold_reason_name(HOLD_PLANE_CHANGED), "plane-changed") == 0);
	CHECK(strcmp(hold_redraw_name(HOLD_REDRAW_B), "yes-B") == 0);
}

int main(void)
{
	test_hash();
	test_mismatch();
	test_frcnt();
	test_regs();
	test_clamp();
	test_judge();
	printf("hold_check_test: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
