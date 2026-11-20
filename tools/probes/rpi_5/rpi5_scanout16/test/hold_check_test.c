/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of src/hold_check.h: every pass and fail path of the hold verdict.
 * Built and run by hold_check_selftest.sh (-I the rpi5_scanout headers).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "pattern.h"
#include "scanout_alive.h"
#include "../src/hold_check.h"

static int fails, checks;

#define CHECK(c)                                                                  \
	do {                                                                      \
		checks++;                                                         \
		if (!(c)) {                                                       \
			fails++;                                                  \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);       \
		}                                                                 \
	} while (0)

#define W 64u
#define H 16u
#define PITCH (W * 2u + 8u) /* a pitch wider than the pixels */
#define TEXT_ROWS 4u

/* text band: a fixed value per row; below it the pattern */
static uint32_t expect_cb(void *ctx, uint32_t x, uint32_t y)
{
	(void)ctx;
	if (y < TEXT_ROWS)
		return 0x1000u + y;
	return pattern_pixel(x, y, W, PATTERN_RGB565);
}

static void fill(uint8_t *pl)
{
	memset(pl, 0xee, (size_t)PITCH * H);
	for (uint32_t y = 0; y < H; y++)
		for (uint32_t x = 0; x < W; x++) {
			uint32_t v = expect_cb(NULL, x, y);

			pl[(size_t)y * PITCH + x * 2u] = (uint8_t)v;
			pl[(size_t)y * PITCH + x * 2u + 1u] = (uint8_t)(v >> 8);
		}
}

/* a good sample series: n samples, FRCNT from f0 up by 15 each */
#define PVC 0x1u
#define PVV 0x1u
static void series(struct hc_sample *s, uint32_t n, uint32_t f0)
{
	for (uint32_t i = 0; i < n; i++) {
		s[i].status = (((f0 + i * 15u) & 0x3fu) << 16) | 0x2000u | 0x123u;
		s[i].dl_ptr = 0x20u;
		s[i].pv_control = PVC | 0x80u;
		s[i].pv_vcontrol = PVV | 0x10u;
		s[i].list_hash = 0xabcdu;
		s[i].plane_hash = 0x1234u;
		s[i].rows_touched = 0;
	}
}

static void test_constants(void)
{
	CHECK(HC_FRCNT_SHIFT == SA_STATUS_FRCNT_SHIFT);
	CHECK(HC_FRCNT_MASK == SA_STATUS_FRCNT_MASK);
	CHECK(HC_PV_CONTROL_EN == SA_PV_CONTROL_EN);
	CHECK(HC_PV_VCONTROL_VIDEN == SA_PV_VCONTROL_VIDEN);
}

static void test_fnv(void)
{
	CHECK(hc_fnv1a("", 0) == 0x811c9dc5u);
	CHECK(hc_fnv1a("a", 1) == 0xe40c292cu);
	CHECK(hc_fnv1a("foobar", 6) == 0xbf9cf968u);
	CHECK(hc_fnv1a_update(hc_fnv1a("foo", 3), (const uint8_t *)"bar", 3) == 0xbf9cf968u);
}

static void test_rows_hash(void)
{
	static uint8_t a[PITCH * H], b[PITCH * H];

	fill(a);
	memcpy(b, a, sizeof a);
	CHECK(hc_rows_hash(a, PITCH, H, 4) == hc_rows_hash(b, PITCH, H, 4));
	CHECK(hc_row_at(0, 4, H) == 0 && hc_row_at(1, 4, H) == 4 && hc_row_at(3, 4, H) == 12);
	/* a change in a sampled row is seen, in a skipped row it is not */
	b[(size_t)4 * PITCH + 10] ^= 1;
	CHECK(hc_rows_hash(a, PITCH, H, 4) != hc_rows_hash(b, PITCH, H, 4));
	memcpy(b, a, sizeof a);
	b[(size_t)5 * PITCH + 10] ^= 1;
	CHECK(hc_rows_hash(a, PITCH, H, 4) == hc_rows_hash(b, PITCH, H, 4));
	CHECK(hc_rows_hash(a, PITCH, H, H) != hc_rows_hash(b, PITCH, H, H));
	/* the padding at the end of a row is part of the row */
	memcpy(b, a, sizeof a);
	b[PITCH - 1] ^= 1;
	CHECK(hc_rows_hash(a, PITCH, H, 4) != hc_rows_hash(b, PITCH, H, 4));
	CHECK(hc_rows_changed(a, b, PITCH, H) == 1);
	memcpy(b, a, sizeof a);
	CHECK(hc_rows_changed(a, b, PITCH, H) == 0);
	for (uint32_t i = 0; i < sizeof b; i++)
		b[i] = (uint8_t)~a[i];
	CHECK(hc_rows_changed(a, b, PITCH, H) == H);
	CHECK(hc_rows_hash(a, PITCH, H, 0) == HC_FNV_BASIS);
}

static void test_mismatch(void)
{
	static uint8_t a[PITCH * H];
	struct hc_mismatch m;

	fill(a);
	m = hc_mismatch_scan(a, PITCH, W, H, 2, expect_cb, NULL);
	CHECK(m.checked == W * H && m.count == 0 && m.rows == 0);

	/* a single pixel: row 9, column 17 (low byte only) */
	a[(size_t)9 * PITCH + 17 * 2] ^= 0x01;
	m = hc_mismatch_scan(a, PITCH, W, H, 2, expect_cb, NULL);
	CHECK(m.count == 1 && m.first_x == 17 && m.first_y == 9 && m.rows == 1);

	/* a second one in the same row, one in the text band: first is the text one */
	a[(size_t)9 * PITCH + 40 * 2 + 1] ^= 0x80;
	a[(size_t)2 * PITCH + 63 * 2] ^= 0x04;
	m = hc_mismatch_scan(a, PITCH, W, H, 2, expect_cb, NULL);
	CHECK(m.count == 3 && m.first_x == 63 && m.first_y == 2 && m.rows == 2);

	/* the whole screen changed */
	memset(a, 0, sizeof a);
	m = hc_mismatch_scan(a, PITCH, W, H, 2, expect_cb, NULL);
	CHECK(m.rows == H && m.first_x == 0 && m.first_y == 0 && m.count > W * H / 2);

	/* padding bytes are not pixels */
	fill(a);
	a[W * 2 + 3] ^= 0xff;
	m = hc_mismatch_scan(a, PITCH, W, H, 2, expect_cb, NULL);
	CHECK(m.count == 0);
}

static uint32_t expect32(void *ctx, uint32_t x, uint32_t y)
{
	(void)ctx;
	return pattern_pixel(x, y, 8, PATTERN_XRGB8888);
}

static void test_mismatch32(void)
{
	static uint8_t a[8 * 4 * 2];
	struct hc_mismatch m;

	for (uint32_t y = 0; y < 2; y++)
		for (uint32_t x = 0; x < 8; x++) {
			uint32_t v = expect32(NULL, x, y);

			memcpy(&a[(y * 8 + x) * 4], &v, 4); /* host is little-endian */
		}
	m = hc_mismatch_scan(a, 32, 8, 2, 4, expect32, NULL);
	CHECK(m.count == 0);
	a[(8 + 3) * 4 + 3] = 0x7f; /* the X byte */
	m = hc_mismatch_scan(a, 32, 8, 2, 4, expect32, NULL);
	CHECK(m.count == 1 && m.first_x == 3 && m.first_y == 1 && m.rows == 1);
}

static void test_frcnt(void)
{
	CHECK(hc_frcnt(0x003f0000u) == 63 && hc_frcnt(0xffffffffu) == 63);
	CHECK(hc_frcnt_delta(10u << 16, 25u << 16) == 15);
	CHECK(hc_frcnt_delta(63u << 16, 0u) == 1);	/* wraps */
	CHECK(hc_frcnt_delta(60u << 16, 11u << 16) == 15);
	CHECK(hc_frcnt_delta(7u << 16, 7u << 16) == 0);	/* stalled */
	CHECK(hc_frcnt_delta(7u << 16, (7u << 16) | 0x1fffu) == 0); /* only YLINE moved */
}

static void test_clamp(void)
{
	struct hc_clamp c;

	/* hold_s 60: limit 50000; 3000 + 20000 fits */
	c = hc_clamp_hold(3000, 20000, 60);
	CHECK(c.end_ms == 23000 && c.hold_ms == 20000 && !c.clamped);
	/* the clamp shortens the hold */
	c = hc_clamp_hold(40000, 20000, 60);
	CHECK(c.end_ms == 50000 && c.hold_ms == 10000 && c.clamped);
	/* exactly at the limit is not clamped */
	c = hc_clamp_hold(30000, 20000, 60);
	CHECK(c.end_ms == 50000 && !c.clamped);
	/* the limit is already past: no hold, reported */
	c = hc_clamp_hold(55000, 20000, 60);
	CHECK(c.end_ms == 55000 && c.hold_ms == 0 && c.clamped);
	/* a watchdog shorter than the margin */
	c = hc_clamp_hold(0, 20000, 5);
	CHECK(c.end_ms == 0 && c.hold_ms == 0 && c.clamped);
	/* a huge hold_s does not overflow the limit */
	c = hc_clamp_hold(1000, 20000, 0xffffffffu);
	CHECK(c.end_ms == 21000 && !c.clamped);
}

static void test_flags(void)
{
	CHECK(hc_unclocked(0xffffffffu) && !hc_unclocked(0xfffffffeu) && !hc_unclocked(0));
	CHECK(hc_pv_running(1, 1) && hc_pv_running(0x81, 0x11));
	CHECK(!hc_pv_running(0, 1) && !hc_pv_running(1, 0) && !hc_pv_running(0xfe, 0xfe));
}

static void test_verdict(void)
{
	struct hc_sample s[80];
	struct hc_verdict v;
	uint32_t n = 80; /* 20 s at 250 ms */

	/* pass, FRCNT starting near the wrap so it passes 63 -> 0 */
	series(s, n, 50);
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.pass && v.reason == HC_PASS && v.index == 0 && !v.redraw && v.redraw_phase == 0);
	CHECK(hc_frcnt(s[1].status) == 1 && hc_frcnt(s[0].status) == 50); /* 50+15 wrapped */

	/* a one-sample series passes (baseline only) */
	series(s, 1, 0);
	v = hc_verdict_make(s, 1, 1, 1);
	CHECK(v.pass);
	v = hc_verdict_make(s, 0, 1, 1);
	CHECK(v.pass);

	/* plane changed at sample 33, 1 row touched */
	series(s, n, 3);
	for (uint32_t i = 33; i < n; i++) {
		s[i].plane_hash ^= 1;
		s[i].rows_touched = 1;
	}
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.pass && v.reason == HC_PLANE_CHANGED && v.index == 33 && v.rows_touched == 1);
	CHECK(v.redraw && v.redraw_phase == 'B');

	/* the whole screen */
	series(s, n, 3);
	s[5].plane_hash = 0;
	s[5].rows_touched = 1080;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_PLANE_CHANGED && v.index == 5 && v.rows_touched == 1080);

	/* list changed (and the plane too: the list outranks it) */
	series(s, n, 3);
	s[10].list_hash ^= 4;
	s[10].plane_hash ^= 4;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.pass && v.reason == HC_LIST_CHANGED && v.index == 10 && !v.redraw);

	/* pointer changed (outranks the list) */
	series(s, n, 3);
	s[12].dl_ptr = 0x40;
	s[12].list_hash ^= 4;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_PTR_CHANGED && v.index == 12);

	/* frcnt stalled: sample 20 repeats sample 19's count */
	series(s, n, 3);
	s[20].status = s[19].status;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.pass && v.reason == HC_FRCNT_STALLED && v.index == 20);
	/* a stall in a pair that wraps */
	series(s, n, 3);
	s[2].status = (63u << 16);
	s[3].status = (63u << 16);
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_FRCNT_STALLED && v.index == 3);

	/* PV disabled: EN clear, then VIDEN clear */
	series(s, n, 3);
	s[7].pv_control = 0x80;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.pass && v.reason == HC_PV_DISABLED && v.index == 7);
	series(s, n, 3);
	s[0].pv_vcontrol = 0;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_PV_DISABLED && v.index == 0);

	/* unclocked: all ones in any register, outranks PV (all ones has EN set,
	 * and a stalled count) */
	series(s, n, 3);
	s[9].status = 0xffffffffu;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.pass && v.reason == HC_UNCLOCKED && v.index == 9);
	series(s, n, 3);
	s[0].pv_control = 0xffffffffu;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_UNCLOCKED && v.index == 0);
	series(s, n, 3);
	s[4].dl_ptr = 0xffffffffu;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_UNCLOCKED && v.index == 4);

	/* the first failing sample wins over a later, higher-ranked one */
	series(s, n, 3);
	s[6].plane_hash ^= 1;
	s[30].status = 0xffffffffu;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(v.reason == HC_PLANE_CHANGED && v.index == 6);

	/* redraw A: the plane changed between the adopt and our write; the hold passes */
	series(s, n, 3);
	v = hc_verdict_make(s, n, 7, 8);
	CHECK(v.pass && v.redraw && v.redraw_phase == 'A');
	/* both A and B: A is reported, the hold still fails */
	s[2].plane_hash ^= 1;
	v = hc_verdict_make(s, n, 7, 8);
	CHECK(!v.pass && v.reason == HC_PLANE_CHANGED && v.redraw_phase == 'A');
	/* a failure other than the plane is no redraw */
	series(s, n, 3);
	s[2].list_hash ^= 1;
	v = hc_verdict_make(s, n, 7, 7);
	CHECK(!v.redraw && v.redraw_phase == 0);
}

static void test_names(void)
{
	CHECK(!strcmp(hc_reason_name(HC_PASS), "pass"));
	CHECK(!strcmp(hc_reason_name(HC_UNCLOCKED), "unclocked"));
	CHECK(!strcmp(hc_reason_name(HC_PV_DISABLED), "pv disabled"));
	CHECK(!strcmp(hc_reason_name(HC_FRCNT_STALLED), "frcnt stalled"));
	CHECK(!strcmp(hc_reason_name(HC_PTR_CHANGED), "pointer changed"));
	CHECK(!strcmp(hc_reason_name(HC_LIST_CHANGED), "list changed"));
	CHECK(!strcmp(hc_reason_name(HC_PLANE_CHANGED), "plane changed"));
}

int main(void)
{
	test_constants();
	test_fnv();
	test_rows_hash();
	test_mismatch();
	test_mismatch32();
	test_frcnt();
	test_clamp();
	test_flags();
	test_verdict();
	test_names();
	printf("hold_check_test: %d checks, %d failed\n", checks, fails);
	return fails ? 1 : 0;
}
