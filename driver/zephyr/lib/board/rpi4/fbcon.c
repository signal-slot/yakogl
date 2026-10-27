/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * printk into the framebuffer (see fbcon.h). The mapping is uncached, so there
 * is no cache maintenance; when the text area is full it wraps to the top and
 * clears the line it is about to write.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk-hooks.h>
#include <string.h>

#include "gc_font8x16.h"
#include "fbcon.h"

static struct {
	uint8_t *fb;
	uint32_t width, pitch, rows_px, scale;
	uint32_t cols, rows, col, row;
} c;

#define FG 0x00ffffffu
#define BG 0x00202020u

static void fill_line(uint32_t row)
{
	uint32_t h = GC_FONT_H * c.scale;
	for (uint32_t y = row * h; y < (row + 1) * h && y < c.rows_px; y++) {
		uint32_t *p = (uint32_t *)(c.fb + (size_t)y * c.pitch);
		for (uint32_t x = 0; x < c.width; x++)
			p[x] = BG;
	}
}

static void glyph(uint32_t col, uint32_t row, int ch)
{
	if (ch < GC_FONT_FIRST || ch > GC_FONT_LAST)
		ch = '?';
	const uint8_t *g = gc_font8x16[ch - GC_FONT_FIRST];
	for (uint32_t gy = 0; gy < GC_FONT_H; gy++) {
		for (uint32_t sy = 0; sy < c.scale; sy++) {
			uint32_t y = (row * GC_FONT_H + gy) * c.scale + sy;
			uint32_t *p = (uint32_t *)(c.fb + (size_t)y * c.pitch);
			for (uint32_t gx = 0; gx < GC_FONT_W; gx++) {
				uint32_t v = (g[gy] & (0x80 >> gx)) ? FG : BG;
				for (uint32_t sx = 0; sx < c.scale; sx++)
					p[(col * GC_FONT_W + gx) * c.scale + sx] = v;
			}
		}
	}
}

static int out(int ch)
{
	if (ch == '\r')
		return ch;
	if (ch == '\n' || c.col >= c.cols) {
		c.col = 0;
		c.row = (c.row + 1) % c.rows;
		fill_line(c.row);
		if (ch == '\n')
			return ch;
	}
	glyph(c.col++, c.row, ch);
	return ch;
}

void fbcon_attach(uint8_t *fb, uint32_t width, uint32_t pitch, uint32_t rows_px, uint32_t scale)
{
	c.fb = fb;
	c.width = width;
	c.pitch = pitch;
	c.rows_px = rows_px;
	c.scale = scale ? scale : 1;
	c.cols = width / (GC_FONT_W * c.scale);
	c.rows = rows_px / (GC_FONT_H * c.scale);
	c.col = c.row = 0;
	for (uint32_t r = 0; r < c.rows; r++)
		fill_line(r);
	__printk_hook_install(out);
}
