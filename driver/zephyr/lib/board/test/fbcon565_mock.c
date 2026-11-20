// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/*
 * Host test of driver/zephyr/lib/board/rpi5_scanout/fbcon565.h: glyph pixels
 * against the font, RGB565 packing, a padded pitch whose padding stays
 * untouched, scrolling, clipping at the right and bottom edges, the refusals
 * by name, and the foreground pixel count of a string. Built by
 * tools/v3d/check71.d/fbcon565.sh.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fbcon565.h"

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

#define FG 0xF81Fu
#define BG 0x0841u
#define PADB 0xA5u

struct tb {
	uint8_t *mem;
	struct yd5_fc565 f;
	size_t size;
};

static struct tb tb_new(uint32_t w, uint32_t pitch, uint32_t rows)
{
	struct tb t;

	t.size = (size_t)pitch * rows;
	t.mem = malloc(t.size);
	memset(t.mem, PADB, t.size); /* everything starts as the pad marker */
	t.f.buf = (uint16_t *)t.mem;
	t.f.width = w;
	t.f.pitch = pitch;
	t.f.rows_px = rows;
	return t;
}

static uint16_t px(const struct tb *t, uint32_t x, uint32_t y)
{
	return yd5_fc565_row(&t->f, y)[x];
}

/* Every byte of every row past width * 2 still holds the marker. */
static int pad_clean(const struct tb *t)
{
	for (uint32_t y = 0; y < t->f.rows_px; y++)
		for (uint32_t b = t->f.width * 2; b < t->f.pitch; b++)
			if (t->mem[(size_t)y * t->f.pitch + b] != PADB)
				return 0;
	return 1;
}

static uint32_t glyph_ones(char c)
{
	uint32_t n = 0;

	for (int y = 0; y < GC_FONT_H; y++)
		for (int b = 0; b < 8; b++)
			n += (gc_font8x16[(uint8_t)c - GC_FONT_FIRST][y] >> b) & 1u;
	return n;
}

static void test_pack(void)
{
	CHECK(yd5_fc565_pack(0, 0, 0) == 0x0000);
	CHECK(yd5_fc565_pack(255, 255, 255) == 0xFFFF);
	CHECK(yd5_fc565_pack(255, 0, 0) == 0xF800);
	CHECK(yd5_fc565_pack(0, 255, 0) == 0x07E0);
	CHECK(yd5_fc565_pack(0, 0, 255) == 0x001F);
	CHECK(yd5_fc565_pack(255, 0, 255) == 0xF81F);
	CHECK(yd5_fc565_pack(7, 3, 7) == 0x0000); /* low bits truncate */
	CHECK(yd5_fc565_pack(8, 4, 8) == 0x0821);
}

static void test_glyph(void)
{
	struct tb t = tb_new(32, 64, 40);
	uint32_t n = 99;
	char c = 'A';

	CHECK(yd5_fc565_glyph(&t.f, 0, 40, 4, 8, c, 1, FG, BG, &n) == 0);
	CHECK(n == glyph_ones(c));
	for (uint32_t gy = 0; gy < GC_FONT_H; gy++)
		for (uint32_t gx = 0; gx < GC_FONT_W; gx++) {
			int on = (gc_font8x16[c - GC_FONT_FIRST][gy] >> (7 - gx)) & 1;

			CHECK(px(&t, 4 + gx, 8 + gy) == (on ? FG : BG));
		}
	CHECK(px(&t, 3, 8) == 0xA5A5 && px(&t, 12, 8) == 0xA5A5); /* outside the cell */
	CHECK(px(&t, 4, 7) == 0xA5A5 && px(&t, 4, 24) == 0xA5A5);
	CHECK(pad_clean(&t));

	/* scale 2: every font pixel is a 2x2 block, 4x the foreground count */
	memset(t.mem, PADB, t.size);
	CHECK(yd5_fc565_glyph(&t.f, 0, 40, 0, 0, c, 2, FG, BG, &n) == 0);
	CHECK(n == 4 * glyph_ones(c));
	for (uint32_t y = 0; y < 32; y++)
		for (uint32_t x = 0; x < 16; x++) {
			int on = (gc_font8x16[c - GC_FONT_FIRST][y / 2] >> (7 - x / 2)) & 1;

			CHECK(px(&t, x, y) == (on ? FG : BG));
		}

	/* a character outside the font draws as '?' */
	CHECK(yd5_fc565_glyph(&t.f, 0, 40, 0, 0, '\x01', 1, FG, BG, &n) == 0);
	CHECK(n == glyph_ones('?'));
	CHECK(yd5_fc565_glyph(&t.f, 0, 40, 0, 0, (char)0xE9, 1, FG, BG, &n) == 0);
	CHECK(n == glyph_ones('?'));
	free(t.mem);
}

static void test_pitch(void)
{
	struct tb t = tb_new(24, 24 * 2 + 40, 20);
	uint32_t n;

	CHECK(yd5_fc565_clear(&t.f, 0, 20, BG) == 0);
	for (uint32_t y = 0; y < 20; y++)
		for (uint32_t x = 0; x < 24; x++)
			CHECK(px(&t, x, y) == BG);
	CHECK(pad_clean(&t));
	CHECK(yd5_fc565_puts(&t.f, 0, 20, 0, 0, "abc", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 20, 1, 0, "zzzzzzzz", 1, FG, BG, &n) == 0);
	CHECK(pad_clean(&t));
	CHECK(yd5_fc565_scroll(&t.f, 0, 20, 1, BG) == 0);
	CHECK(pad_clean(&t));
	free(t.mem);
}

static void test_scroll(void)
{
	/* band of rows 16..63 inside a 80-row buffer; rows outside stay put */
	struct tb t = tb_new(16, 32, 80);
	uint32_t n;

	CHECK(yd5_fc565_clear(&t.f, 0, 80, BG) == 0);
	CHECK(yd5_fc565_puts(&t.f, 16, 48, 0, 0, "AB", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&t.f, 16, 48, 0, 1, "CD", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&t.f, 16, 48, 0, 2, "EF", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 16, 0, 0, "ZZ", 1, FG, BG, &n) == 0); /* above */

	struct tb ref = tb_new(16, 32, 80);

	CHECK(yd5_fc565_clear(&ref.f, 0, 80, BG) == 0);
	CHECK(yd5_fc565_puts(&ref.f, 0, 16, 0, 0, "ZZ", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&ref.f, 16, 48, 0, 0, "CD", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_puts(&ref.f, 16, 48, 0, 1, "EF", 1, FG, BG, &n) == 0);

	CHECK(yd5_fc565_scroll(&t.f, 16, 48, 1, BG) == 0);
	for (uint32_t y = 0; y < 80; y++)
		for (uint32_t x = 0; x < 16; x++)
			CHECK(px(&t, x, y) == px(&ref, x, y));
	CHECK(pad_clean(&t));

	/* diff counts: t vs the scrolled-back-less original text row */
	uint32_t dpx, cells, rows;

	CHECK(yd5_fc565_diff(&t.f, &ref.f, 0, 80, 1, &dpx, &cells, &rows) == 0);
	CHECK(dpx == 0 && cells == 0 && rows == 0);
	CHECK(yd5_fc565_puts(&ref.f, 0, 80, 1, 3, "Q", 1, FG, BG, &n) == 0);
	CHECK(yd5_fc565_diff(&t.f, &ref.f, 0, 80, 1, &dpx, &cells, &rows) == 0);
	CHECK(dpx == n && cells == 1 && rows == 1);
	CHECK(yd5_fc565_diff(&t.f, &ref.f, 0, 48, 1, &dpx, &cells, &rows) == 0);
	CHECK(dpx == 0 && cells == 0 && rows == 0); /* the change is below row 48 */

	/* a band no taller than one text row is just cleared */
	CHECK(yd5_fc565_scroll(&t.f, 0, 16, 1, FG) == 0);
	CHECK(px(&t, 0, 0) == FG && px(&t, 15, 15) == FG && px(&t, 0, 16) != FG);

	/* scale 2 scrolls by 32 rows */
	struct tb s2 = tb_new(16, 32, 64);

	CHECK(yd5_fc565_clear(&s2.f, 0, 64, BG) == 0);
	CHECK(yd5_fc565_puts(&s2.f, 0, 64, 0, 1, "X", 2, FG, BG, &n) == 0);
	CHECK(yd5_fc565_scroll(&s2.f, 0, 64, 2, BG) == 0);
	struct tb x2 = tb_new(16, 32, 64);

	CHECK(yd5_fc565_clear(&x2.f, 0, 64, BG) == 0);
	CHECK(yd5_fc565_puts(&x2.f, 0, 64, 0, 0, "X", 2, FG, BG, &n) == 0);
	CHECK(memcmp(s2.mem, x2.mem, s2.size) == 0);
	free(t.mem);
	free(ref.mem);
	free(s2.mem);
	free(x2.mem);
}

static void test_clip(void)
{
	/* 20 wide: the second cell at x=8..15 fits, the third (16..23) is cut at 20 */
	struct tb t = tb_new(20, 48, 24);
	uint32_t n;

	CHECK(yd5_fc565_clear(&t.f, 0, 24, BG) == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 24, 0, 0, "HHH", 1, FG, BG, &n) == 0);
	{
		uint32_t exp = 2 * glyph_ones('H');

		for (uint32_t gy = 0; gy < 16; gy++)
			for (uint32_t gx = 0; gx < 4; gx++)
				exp += (gc_font8x16['H' - GC_FONT_FIRST][gy] >> (7 - gx)) & 1u;
		CHECK(n == exp);
	}
	CHECK(pad_clean(&t)); /* nothing spilled into the padding or the next row */

	/* bottom: band of 24 rows, a cell on row 1 (y 16..31) is cut at 24 */
	CHECK(yd5_fc565_clear(&t.f, 0, 24, BG) == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 24, 0, 1, "H", 1, FG, BG, &n) == 0);
	{
		uint32_t exp = 0;

		for (uint32_t gy = 0; gy < 8; gy++)
			exp += (uint32_t)__builtin_popcount(
				gc_font8x16['H' - GC_FONT_FIRST][gy]);
		CHECK(n == exp);
	}
	CHECK(pad_clean(&t));
	/* a cell wholly outside draws nothing and is not an error */
	CHECK(yd5_fc565_puts(&t.f, 0, 24, 0, 5, "H", 1, FG, BG, &n) == 0 && n == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 24, 9, 0, "H", 1, FG, BG, &n) == 0 && n == 0);
	CHECK(pad_clean(&t));

	/* the band ends above the buffer end: rows below the band stay untouched */
	CHECK(yd5_fc565_clear(&t.f, 0, 24, BG) == 0);
	{
		struct tb u = tb_new(20, 48, 32);

		CHECK(yd5_fc565_glyph(&u.f, 0, 24, 0, 16, 'H', 1, FG, BG, &n) == 0);
		CHECK(px(&u, 0, 24) == 0xA5A5 && px(&u, 3, 31) == 0xA5A5);
		free(u.mem);
	}
	free(t.mem);
}

static void test_refusals(void)
{
	struct tb t = tb_new(16, 32, 32);
	struct yd5_fc565 f = t.f;
	uint32_t n = 7;

	CHECK(yd5_fc565_check(&f, 0, 32) == YD5_FC565_OK);
	CHECK(yd5_fc565_check(NULL, 0, 1) == YD5_FC565_E_BUF);
	f.buf = NULL;
	CHECK(yd5_fc565_clear(&f, 0, 1, 0) == YD5_FC565_E_BUF);
	f = t.f;
	f.width = 0;
	CHECK(yd5_fc565_clear(&f, 0, 1, 0) == YD5_FC565_E_WIDTH);
	f = t.f;
	f.pitch = 30; /* < width * 2 */
	CHECK(yd5_fc565_clear(&f, 0, 1, 0) == YD5_FC565_E_PITCH);
	f.pitch = 33; /* odd */
	CHECK(yd5_fc565_check(&f, 0, 1) == YD5_FC565_E_PITCH);
	f = t.f;
	CHECK(yd5_fc565_clear(&f, 0, 0, 0) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_clear(&f, 32, 1, 0) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_clear(&f, 31, 2, 0) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_clear(&f, 1, 0xFFFFFFFFu, 0) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_scroll(&f, 0, 33, 1, 0) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_scroll(&f, 0, 32, 0, 0) == YD5_FC565_E_SCALE);
	CHECK(yd5_fc565_scroll(&f, 0, 32, YD5_FC565_MAX_SCALE + 1, 0) == YD5_FC565_E_SCALE);
	CHECK(yd5_fc565_glyph(&f, 0, 32, 0, 0, 'a', 0, FG, BG, &n) == YD5_FC565_E_SCALE);
	CHECK(n == 0);
	CHECK(yd5_fc565_glyph(&f, 0, 40, 0, 0, 'a', 1, FG, BG, &n) == YD5_FC565_E_BAND);
	CHECK(yd5_fc565_puts(&f, 0, 32, 0, 0, NULL, 1, FG, BG, &n) == YD5_FC565_E_ARG);
	CHECK(yd5_fc565_puts(&f, 0, 32, 0, 0, "a", 9, FG, BG, &n) == YD5_FC565_E_SCALE);
	CHECK(yd5_fc565_diff(&f, NULL, 0, 32, 1, &n, NULL, NULL) == YD5_FC565_E_BUF);
	{
		struct yd5_fc565 g = t.f;

		g.width = 8;
		CHECK(yd5_fc565_diff(&f, &g, 0, 32, 1, &n, NULL, NULL) == YD5_FC565_E_WIDTH);
		CHECK(yd5_fc565_diff(&f, &f, 0, 32, 0, &n, NULL, NULL) == YD5_FC565_E_SCALE);
	}
	/* a refused call writes nothing */
	for (size_t i = 0; i < t.size; i++)
		CHECK(t.mem[i] == PADB);
	CHECK(strcmp(yd5_fc565_err_name(YD5_FC565_E_PITCH), "E_PITCH") == 0);
	CHECK(strcmp(yd5_fc565_err_name(YD5_FC565_E_BAND), "E_BAND") == 0);
	CHECK(strcmp(yd5_fc565_err_name(YD5_FC565_OK), "ok") == 0);
	CHECK(strcmp(yd5_fc565_err_name(-99), "E_?") == 0);
	free(t.mem);
}

static void test_string_count(void)
{
	struct tb t = tb_new(1920, 3840, 64);
	const char *s = "Pi5 RGB565 fbcon: hello, world!";
	uint32_t n = 0, exp = 0, d, cells, rows;
	struct tb before = tb_new(1920, 3840, 64);

	CHECK(yd5_fc565_clear(&t.f, 0, 64, BG) == 0);
	CHECK(yd5_fc565_clear(&before.f, 0, 64, BG) == 0);
	for (const char *p = s; *p; p++)
		exp += glyph_ones(*p);
	CHECK(yd5_fc565_puts(&t.f, 0, 64, 2, 1, s, 1, FG, BG, &n) == 0);
	CHECK(n == exp);
	/* really counted from the buffer, not just the return value */
	{
		uint32_t cnt = 0;

		for (uint32_t y = 0; y < 64; y++)
			for (uint32_t x = 0; x < 1920; x++)
				cnt += px(&t, x, y) == FG;
		CHECK(cnt == exp);
	}
	/* scale 2: 4x; 'text rows changed' is 1 (one text row), cells = printing chars */
	CHECK(yd5_fc565_clear(&t.f, 0, 64, BG) == 0);
	CHECK(yd5_fc565_puts(&t.f, 0, 64, 0, 1, s, 2, FG, BG, &n) == 0);
	CHECK(n == 4 * exp);
	CHECK(yd5_fc565_diff(&before.f, &t.f, 0, 64, 2, &d, &cells, &rows) == 0);
	CHECK(d == 4 * exp && rows == 1);
	{
		uint32_t ink = 0;

		for (const char *p = s; *p; p++)
			ink += glyph_ones(*p) != 0;
		CHECK(cells == ink);
	}
	free(t.mem);
	free(before.mem);
}

int main(void)
{
	test_pack();
	test_glyph();
	test_pitch();
	test_scroll();
	test_clip();
	test_refusals();
	test_string_count();
	printf("fbcon565_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
