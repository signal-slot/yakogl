/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * 16 bpp (RGB565) text renderer for the Pi 5 stage 1 scanout: the firmware's
 * plane is 1920x1080 RGB565 (docs/notes/rpi5-display.md), the Pi 4's fbcon
 * (driver/zephyr/lib/board/rpi4/fbcon.c) draws 32 bpp only. Header-only C99,
 * no Zephyr includes, no printk hook (the probe hooks printk and feeds
 * yd5_fc565_puts).
 *
 * Font: the 8x16 cell of common/gc_font8x16.h (GC_FONT_W/GC_FONT_H, bit 7 =
 * leftmost pixel). A character outside 0x20..0x7e draws as '?'. Glyphs are
 * opaque (foreground and background both written), scaled by an integer
 * factor 1..YD5_FC565_MAX_SCALE.
 *
 * Geometry: yd5_fc565 names the caller's buffer: width in pixels, pitch in
 * bytes (>= width * 2, even), rows_px rows. The pitch padding bytes are never
 * written. A "band" is rows y0 .. y0 + h - 1 of the buffer. Drawing clips at
 * the right edge and the bottom of the band. Nothing asserts: every function
 * returns a YD5_FC565_E_* code (0 = ok), named by yd5_fc565_err_name().
 */
#ifndef RPI5_SCANOUT_FBCON565_H
#define RPI5_SCANOUT_FBCON565_H

#include <stddef.h>
#include <stdint.h>

#include "../common/gc_font8x16.h"

#define YD5_FC565_MAX_SCALE 8u

#define YD5_FC565_OK 0
#define YD5_FC565_E_BUF (-1)   /* null buffer */
#define YD5_FC565_E_WIDTH (-2) /* width 0, or two buffers of different width */
#define YD5_FC565_E_PITCH (-3) /* pitch < width * 2, or odd */
#define YD5_FC565_E_BAND (-4)  /* band empty or beyond rows_px */
#define YD5_FC565_E_SCALE (-5) /* scale 0 or above the maximum */
#define YD5_FC565_E_ARG (-6)   /* null string */

struct yd5_fc565 {
	uint16_t *buf;
	uint32_t width;   /* pixels */
	uint32_t pitch;   /* bytes per row */
	uint32_t rows_px; /* rows in the buffer */
};

static inline const char *yd5_fc565_err_name(int e)
{
	switch (e) {
	case YD5_FC565_OK: return "ok";
	case YD5_FC565_E_BUF: return "E_BUF";
	case YD5_FC565_E_WIDTH: return "E_WIDTH";
	case YD5_FC565_E_PITCH: return "E_PITCH";
	case YD5_FC565_E_BAND: return "E_BAND";
	case YD5_FC565_E_SCALE: return "E_SCALE";
	case YD5_FC565_E_ARG: return "E_ARG";
	default: return "E_?";
	}
}

/* RGB888 channels to RGB565 (truncating, no rounding). */
static inline uint16_t yd5_fc565_pack(uint8_t r, uint8_t g, uint8_t b)
{
	return (uint16_t)(((uint32_t)(r >> 3) << 11) | ((uint32_t)(g >> 2) << 5) | (b >> 3));
}

/* Buffer geometry, and the band y0 .. y0 + h (h must be > 0). */
static inline int yd5_fc565_check(const struct yd5_fc565 *f, uint32_t y0, uint32_t h)
{
	if (!f || !f->buf)
		return YD5_FC565_E_BUF;
	if (f->width == 0)
		return YD5_FC565_E_WIDTH;
	if (f->pitch < f->width * 2u || (f->pitch & 1u))
		return YD5_FC565_E_PITCH;
	if (h == 0 || y0 >= f->rows_px || h > f->rows_px - y0)
		return YD5_FC565_E_BAND;
	return YD5_FC565_OK;
}

static inline uint16_t *yd5_fc565_row(const struct yd5_fc565 *f, uint32_t y)
{
	return (uint16_t *)((uint8_t *)f->buf + (size_t)y * f->pitch);
}

/* Fill the band with one colour (the width pixels of each row only). */
static inline int yd5_fc565_clear(const struct yd5_fc565 *f, uint32_t y0, uint32_t h,
				  uint16_t color)
{
	int e = yd5_fc565_check(f, y0, h);

	if (e)
		return e;
	for (uint32_t y = y0; y < y0 + h; y++) {
		uint16_t *r = yd5_fc565_row(f, y);

		for (uint32_t x = 0; x < f->width; x++)
			r[x] = color;
	}
	return YD5_FC565_OK;
}

/*
 * One glyph with its cell's top-left corner at (x, y), clipped to the width
 * and to the band y0 .. y0 + h. *fg_px (may be NULL) receives the number of
 * foreground pixels written (after clipping).
 */
static inline int yd5_fc565_glyph(const struct yd5_fc565 *f, uint32_t y0, uint32_t h,
				  uint32_t x, uint32_t y, char ch, uint32_t scale,
				  uint16_t fg, uint16_t bg, uint32_t *fg_px)
{
	int e = yd5_fc565_check(f, y0, h);
	uint32_t n = 0, c = (uint8_t)ch;

	if (fg_px)
		*fg_px = 0;
	if (e)
		return e;
	if (scale == 0 || scale > YD5_FC565_MAX_SCALE)
		return YD5_FC565_E_SCALE;
	if (c < GC_FONT_FIRST || c > GC_FONT_LAST)
		c = '?';
	for (uint32_t gy = 0; gy < GC_FONT_H; gy++) {
		uint8_t bits = gc_font8x16[c - GC_FONT_FIRST][gy];

		for (uint32_t sy = 0; sy < scale; sy++) {
			uint32_t py = y + gy * scale + sy;
			uint16_t *r;

			if (py < y0 || py >= y0 + h)
				continue;
			r = yd5_fc565_row(f, py);
			for (uint32_t gx = 0; gx < GC_FONT_W; gx++) {
				uint32_t on = (bits >> (7 - gx)) & 1u;

				for (uint32_t sx = 0; sx < scale; sx++) {
					uint32_t px = x + gx * scale + sx;

					if (px >= f->width)
						continue;
					r[px] = on ? fg : bg;
					n += on;
				}
			}
		}
	}
	if (fg_px)
		*fg_px = n;
	return YD5_FC565_OK;
}

/*
 * A NUL-terminated string at text cell (col, row) of the band starting at y0
 * (cell size 8*scale x 16*scale). '\n' is not interpreted. Characters past
 * the right edge or below the band are clipped. *fg_px as in _glyph, summed.
 */
static inline int yd5_fc565_puts(const struct yd5_fc565 *f, uint32_t y0, uint32_t h,
				 uint32_t col, uint32_t row, const char *s, uint32_t scale,
				 uint16_t fg, uint16_t bg, uint32_t *fg_px)
{
	uint32_t total = 0, cw, ch;
	int e;

	if (fg_px)
		*fg_px = 0;
	if (!s)
		return YD5_FC565_E_ARG;
	if (scale == 0 || scale > YD5_FC565_MAX_SCALE)
		return YD5_FC565_E_SCALE;
	cw = GC_FONT_W * scale;
	ch = GC_FONT_H * scale;
	for (uint32_t i = 0; s[i]; i++) {
		uint32_t n = 0;

		e = yd5_fc565_glyph(f, y0, h, (col + i) * cw, y0 + row * ch, s[i], scale, fg,
				    bg, &n);
		if (e)
			return e;
		total += n;
	}
	if (fg_px)
		*fg_px = total;
	return YD5_FC565_OK;
}

/*
 * Scroll the band up by one text row (16 * scale pixel rows) and clear the
 * freed rows at the bottom to bg. A band no taller than one text row is just
 * cleared.
 */
static inline int yd5_fc565_scroll(const struct yd5_fc565 *f, uint32_t y0, uint32_t h,
				   uint32_t scale, uint16_t bg)
{
	uint32_t step;
	int e = yd5_fc565_check(f, y0, h);

	if (e)
		return e;
	if (scale == 0 || scale > YD5_FC565_MAX_SCALE)
		return YD5_FC565_E_SCALE;
	step = GC_FONT_H * scale;
	if (step >= h)
		return yd5_fc565_clear(f, y0, h, bg);
	for (uint32_t y = y0; y + step < y0 + h; y++) {
		const uint16_t *s = yd5_fc565_row(f, y + step);
		uint16_t *d = yd5_fc565_row(f, y);

		for (uint32_t x = 0; x < f->width; x++)
			d[x] = s[x];
	}
	return yd5_fc565_clear(f, y0 + h - step, step, bg);
}

/*
 * Compare two buffers of the same width over the band: *px receives the
 * number of differing pixels, *cells the number of text cells (8*scale x
 * 16*scale, counted from the band's top-left, edge cells clipped) holding at
 * least one, *rows the number of text rows with a change (the probe's
 * 'text rows <n> changed' line). Any out pointer may be NULL.
 */
static inline int yd5_fc565_diff(const struct yd5_fc565 *a, const struct yd5_fc565 *b,
				 uint32_t y0, uint32_t h, uint32_t scale, uint32_t *px,
				 uint32_t *cells, uint32_t *rows)
{
	uint32_t npx = 0, ncell = 0, nrow = 0, cw, ch, cols;
	int e = yd5_fc565_check(a, y0, h);

	if (px)
		*px = 0;
	if (cells)
		*cells = 0;
	if (rows)
		*rows = 0;
	if (e == YD5_FC565_OK)
		e = yd5_fc565_check(b, y0, h);
	if (e == YD5_FC565_OK && a->width != b->width)
		e = YD5_FC565_E_WIDTH;
	if (e)
		return e;
	if (scale == 0 || scale > YD5_FC565_MAX_SCALE)
		return YD5_FC565_E_SCALE;
	cw = GC_FONT_W * scale;
	ch = GC_FONT_H * scale;
	cols = (a->width + cw - 1) / cw;
	for (uint32_t cy = 0; cy * ch < h; cy++) {
		uint32_t row_hit = 0;

		for (uint32_t cx = 0; cx < cols; cx++) {
			uint32_t hit = 0;

			for (uint32_t y = cy * ch; y < (cy + 1) * ch && y < h; y++) {
				const uint16_t *ra = yd5_fc565_row(a, y0 + y);
				const uint16_t *rb = yd5_fc565_row(b, y0 + y);

				for (uint32_t x = cx * cw; x < (cx + 1) * cw && x < a->width;
				     x++) {
					if (ra[x] != rb[x]) {
						hit = 1;
						npx++;
					}
				}
			}
			ncell += hit;
			row_hit |= hit;
		}
		nrow += row_hit;
	}
	if (px)
		*px = npx;
	if (cells)
		*cells = ncell;
	if (rows)
		*rows = nrow;
	return YD5_FC565_OK;
}

#endif /* RPI5_SCANOUT_FBCON565_H */
