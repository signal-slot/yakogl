/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fb_flush.h: which bytes of the Pi 5 scanout buffer to clean from the data
 * cache after the CPU wrote them. The HVS reads the buffer from memory, not
 * from the Cortex-A76's caches, so bytes written by the CPU reach it only
 * after a clean (DC CVAC, Zephyr sys_cache_data_flush_range) to the point of
 * coherency. Header-only C11, no Zephyr or libc dependency beyond stdint and
 * stddef; the clean itself is a caller-given callback, so the host mock
 * records the calls (driver/zephyr/lib/board/test/fb_flush_mock.c).
 *
 * Cache line: the Cortex-A76 data cache line is 64 bytes (Arm Cortex-A76
 * TRM, "L1 data cache": 64-byte lines). The portable source is CTR_EL0
 * DminLine, bits [19:16], log2 of the smallest data cache line in words
 * (Arm ARM, CTR_EL0): line = 4 << DminLine bytes. fb_flush_line reads it
 * through an injectable function (Zephyr passes an mrs of CTR_EL0, the host
 * mock passes a constant) and falls back to 64 when there is none.
 *
 * Rows: surface of width x height pixels, bpp BYTES per pixel, pitch bytes
 * per row (pitch >= width * bpp; the gap is not cleaned except inside a
 * range that spans rows). A range of rows [y0, y1) covers
 * base + y0*pitch up to base + (y1-1)*pitch + width*bpp.
 *
 * Split screen: the text area (top) and the pattern area (bottom) are two
 * trackers over the same surface, fb_dirty_init_area with their row windows,
 * flushed separately. A line shared by the two areas (an area edge that is
 * not line aligned) is cleaned by both; that is harmless.
 */
#ifndef RPI5_SCANOUT_FB_FLUSH_H
#define RPI5_SCANOUT_FB_FLUSH_H

#include <stddef.h>
#include <stdint.h>

#define FB_FLUSH_LINE_A76 64u
#define FB_DIRTY_MAX 8u

/* returns CTR_EL0 */
typedef uint64_t (*fb_flush_ctr_fn)(void);

struct fb_range {
	uint64_t start; /* line aligned, inclusive */
	uint64_t end;   /* line aligned, exclusive; end == start when empty */
};

/* the data cache line in bytes: from CTR_EL0 DminLine, 64 without a reader */
static inline uint32_t fb_flush_line(fb_flush_ctr_fn ctr)
{
	uint32_t dmin;

	if (!ctr) {
		return FB_FLUSH_LINE_A76;
	}
	dmin = (uint32_t)((ctr() >> 16) & 0xfu);
	return 4u << dmin;
}

/*
 * The line-aligned [start, end) covering [base, base + len). Empty
 * ({0, 0}) for len 0, for a line that is 0 or not a power of two, and when
 * the range or its rounding would wrap the address space.
 */
static inline struct fb_range fb_flush_range(uint64_t base, uint64_t len, uint32_t line)
{
	struct fb_range r = {0, 0};
	uint64_t last, mask;

	if (len == 0 || line == 0 || (line & (line - 1u)) != 0) {
		return r;
	}
	if (base + len < base) {
		return r;
	}
	mask = (uint64_t)line - 1u;
	last = base + len - 1u;
	if ((last | mask) == UINT64_MAX) {
		return r;
	}
	r.start = base & ~mask;
	r.end = (last | mask) + 1u;
	return r;
}

/* called once per range to clean: [addr, addr + len) */
typedef void (*fb_flush_fn)(uint64_t addr, uint64_t len, void *ctx);

struct fb_rows {
	uint32_t y0; /* inclusive */
	uint32_t y1; /* exclusive */
};

struct fb_dirty {
	uint64_t base;
	uint32_t width, height, bpp, pitch;
	uint32_t row0, row1; /* the area's rows: marks are clamped to [row0, row1) */
	uint32_t line;
	uint32_t max_ranges; /* 1..FB_DIRTY_MAX */
	uint32_t n;
	struct fb_rows r[FB_DIRTY_MAX + 1u];
};

/*
 * 0 on success, -1 for a zero dimension, pitch < width*bpp, max_ranges 0 or
 * above FB_DIRTY_MAX, a bad line or an empty row window (row0 >= row1 or
 * row1 > height).
 */
static inline int fb_dirty_init_area(struct fb_dirty *d, uint64_t base, uint32_t width,
				     uint32_t height, uint32_t bpp, uint32_t pitch,
				     uint32_t row0, uint32_t row1, uint32_t line,
				     uint32_t max_ranges)
{
	if (!d || width == 0 || height == 0 || bpp == 0 ||
	    (uint64_t)pitch < (uint64_t)width * bpp || max_ranges == 0 ||
	    max_ranges > FB_DIRTY_MAX || row0 >= row1 || row1 > height || line == 0 ||
	    (line & (line - 1u)) != 0) {
		return -1;
	}
	d->base = base;
	d->width = width;
	d->height = height;
	d->bpp = bpp;
	d->pitch = pitch;
	d->row0 = row0;
	d->row1 = row1;
	d->line = line;
	d->max_ranges = max_ranges;
	d->n = 0;
	return 0;
}

/* the whole surface as the area */
static inline int fb_dirty_init(struct fb_dirty *d, uint64_t base, uint32_t width,
				uint32_t height, uint32_t bpp, uint32_t pitch, uint32_t line,
				uint32_t max_ranges)
{
	return fb_dirty_init_area(d, base, width, height, bpp, pitch, 0, height, line,
				  max_ranges);
}

/*
 * Marks rows [y0, y0 + count) written, clamped to the area. Overlapping and
 * adjacent ranges merge; when more than max_ranges remain, the two ranges
 * with the smallest gap merge (the rows between them are cleaned too).
 * Returns the number of rows marked after clamping.
 */
static inline uint32_t fb_dirty_mark(struct fb_dirty *d, uint32_t y0, uint32_t count)
{
	uint64_t e = (uint64_t)y0 + count;
	uint32_t s, t, i, j, marked;

	if (y0 < d->row0) {
		y0 = d->row0;
	}
	if (e > d->row1) {
		e = d->row1;
	}
	if (count == 0 || y0 >= e) {
		return 0;
	}
	marked = (uint32_t)(e - y0);
	s = y0;
	t = (uint32_t)e;
	/* absorb every range that overlaps or touches [s, t) */
	for (i = 0, j = 0; i < d->n; i++) {
		if (d->r[i].y1 < s || d->r[i].y0 > t) {
			d->r[j++] = d->r[i];
		} else {
			if (d->r[i].y0 < s) {
				s = d->r[i].y0;
			}
			if (d->r[i].y1 > t) {
				t = d->r[i].y1;
			}
		}
	}
	d->n = j;
	/* insert sorted */
	for (i = 0; i < d->n && d->r[i].y0 < s; i++) {
	}
	for (j = d->n; j > i; j--) {
		d->r[j] = d->r[j - 1u];
	}
	d->r[i].y0 = s;
	d->r[i].y1 = t;
	d->n++;
	while (d->n > d->max_ranges) {
		uint32_t best = 0, bg = UINT32_MAX;

		for (i = 0; i + 1u < d->n; i++) {
			uint32_t gap = d->r[i + 1u].y0 - d->r[i].y1;

			if (gap < bg) {
				bg = gap;
				best = i;
			}
		}
		d->r[best].y1 = d->r[best + 1u].y1;
		for (i = best + 1u; i + 1u < d->n; i++) {
			d->r[i] = d->r[i + 1u];
		}
		d->n--;
	}
	return marked;
}

/* the line-aligned bytes of rows [y0, y1) */
static inline struct fb_range fb_dirty_bytes(const struct fb_dirty *d, uint32_t y0, uint32_t y1)
{
	uint64_t len = (uint64_t)(y1 - y0 - 1u) * d->pitch + (uint64_t)d->width * d->bpp;

	return fb_flush_range(d->base + (uint64_t)y0 * d->pitch, len, d->line);
}

/*
 * Calls fn once per range to clean, in address order, and empties the
 * tracker. Ranges whose line-aligned bytes overlap or touch (rows that end
 * in the same line) are passed as one. Returns the number of calls.
 */
static inline uint32_t fb_dirty_flush(struct fb_dirty *d, fb_flush_fn fn, void *ctx)
{
	struct fb_range cur = {0, 0};
	uint32_t i, calls = 0;

	for (i = 0; i < d->n; i++) {
		struct fb_range r = fb_dirty_bytes(d, d->r[i].y0, d->r[i].y1);

		if (r.end == r.start) {
			continue;
		}
		if (cur.end != cur.start && r.start <= cur.end) {
			if (r.end > cur.end) {
				cur.end = r.end;
			}
			continue;
		}
		if (cur.end != cur.start) {
			fn(cur.start, cur.end - cur.start, ctx);
			calls++;
		}
		cur = r;
	}
	if (cur.end != cur.start) {
		fn(cur.start, cur.end - cur.start, ctx);
		calls++;
	}
	d->n = 0;
	return calls;
}

#endif /* RPI5_SCANOUT_FB_FLUSH_H */
