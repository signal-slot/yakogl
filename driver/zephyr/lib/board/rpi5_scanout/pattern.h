/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A known test pattern for a scanout buffer, its readback check and the Pi 4's
 * split-screen layout. Header-only, stdint/stddef only, host-testable
 * (driver/zephyr/lib/board/test/pattern_mock.c).
 *
 * The pattern is eight colour bars across the screen's width (white, yellow,
 * cyan, green, magenta, red, blue, black). In each colour channel the top two
 * bits are the bar and the remaining low bits are a hash of (x, y), so a wrong
 * row, column, pitch or byte order changes the pixel in the bits a bar alone
 * would not show. The pattern depends on the whole screen's width and on
 * absolute (x, y), so a band filled alone equals the same band of a full fill.
 * The X byte of the 32 bpp formats is 0 and is checked too.
 *
 * Pixels are little-endian in memory: XRGB8888 is B, G, R, X bytes;
 * XBGR8888 is R, G, B, X; RGB565 is r5:g6:b5 in one 16-bit word.
 * Buffers are accessed through volatile pointers (device memory, readback).
 */
#ifndef RPI5_SCANOUT_PATTERN_H
#define RPI5_SCANOUT_PATTERN_H

#include <stddef.h>
#include <stdint.h>

enum pattern_fmt {
	PATTERN_XRGB8888 = 0,
	PATTERN_XBGR8888 = 1,
	PATTERN_RGB565 = 2,
};

struct pattern_rect {
	uint32_t x, y, w, h;
};

struct pattern_layout {
	struct pattern_rect text;    /* top band: the console */
	struct pattern_rect pattern; /* bottom band: the pattern */
};

struct pattern_report {
	uint32_t checked;          /* pixels looked at */
	uint32_t mismatches;       /* of those, differing from the pattern */
	uint32_t first_x, first_y; /* of the first mismatch (row-major order) */
	uint32_t expected, got;    /* its pixel values (raw, in the format) */
	uint32_t checksum;         /* FNV-1a over the raw values read, in order */
};

/* The short name for a verdict line, "?" for an unknown format. */
static inline const char *pattern_fmt_name(enum pattern_fmt fmt)
{
	switch (fmt) {
	case PATTERN_XRGB8888:
		return "XRGB8888";
	case PATTERN_XBGR8888:
		return "XBGR8888";
	case PATTERN_RGB565:
		return "RGB565";
	}
	return "?";
}

static inline uint32_t pattern_bytes_per_pixel(enum pattern_fmt fmt)
{
	return fmt == PATTERN_RGB565 ? 2u : 4u;
}

static inline uint32_t pattern_hash(uint32_t x, uint32_t y)
{
	uint32_t h = x * 0x9e3779b1u ^ (y + 1u) * 0x85ebca6bu;

	h ^= h >> 15;
	h *= 0x2c1b3c6du;
	h ^= h >> 12;
	return h;
}

/* One channel of n bits: the bar bit in the top two bits, the hash below. */
static inline uint32_t pattern_chan(unsigned int on, uint32_t h, unsigned int n)
{
	return ((on ? 3u : 0u) << (n - 2u)) | (h & ((1u << (n - 2u)) - 1u));
}

/* The raw pixel value the pattern has at (x, y) of a width-pixel screen. */
static inline uint32_t pattern_pixel(uint32_t x, uint32_t y, uint32_t width,
				     enum pattern_fmt fmt)
{
	static const uint8_t bars[8] = { 7, 6, 3, 2, 5, 4, 1, 0 }; /* bit0 B, 1 G, 2 R */
	uint32_t bar = width ? (uint32_t)(((uint64_t)x * 8u) / width) : 0u;
	uint32_t c = bars[bar > 7u ? 7u : bar];
	uint32_t h = pattern_hash(x, y);

	if (fmt == PATTERN_RGB565) {
		uint32_t r = pattern_chan(c & 4u, h, 5u);
		uint32_t g = pattern_chan(c & 2u, h >> 5, 6u);
		uint32_t b = pattern_chan(c & 1u, h >> 11, 5u);

		return (r << 11) | (g << 5) | b;
	} else {
		uint32_t r = pattern_chan(c & 4u, h, 8u);
		uint32_t g = pattern_chan(c & 2u, h >> 8, 8u);
		uint32_t b = pattern_chan(c & 1u, h >> 16, 8u);

		return fmt == PATTERN_XBGR8888 ? (b << 16) | (g << 8) | r
					       : (r << 16) | (g << 8) | b;
	}
}

static inline uint32_t pattern_load(const volatile uint8_t *p, uint32_t bpp_bytes)
{
	uint32_t v = p[0] | ((uint32_t)p[1] << 8);

	if (bpp_bytes == 4u) {
		v |= ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
	}
	return v;
}

static inline void pattern_store(volatile uint8_t *p, uint32_t bpp_bytes, uint32_t v)
{
	p[0] = (uint8_t)v;
	p[1] = (uint8_t)(v >> 8);
	if (bpp_bytes == 4u) {
		p[2] = (uint8_t)(v >> 16);
		p[3] = (uint8_t)(v >> 24);
	}
}

/*
 * Fill the rectangle r of a width x height screen (the bars span width). The
 * rectangle is clipped to the screen. bpp must be the format's (32 or 16);
 * returns 0, or -1 for a bpp that does not match fmt or a pitch too small.
 */
static inline int pattern_fill_rect(void *buf, uint32_t width, uint32_t height,
				    uint32_t pitch_bytes, uint32_t bpp,
				    enum pattern_fmt fmt, struct pattern_rect r)
{
	uint32_t bp = pattern_bytes_per_pixel(fmt);
	volatile uint8_t *base = (volatile uint8_t *)buf;

	if (!buf || bpp != bp * 8u || pitch_bytes < width * bp) {
		return -1;
	}
	for (uint32_t y = r.y; y < r.y + r.h && y < height; y++) {
		for (uint32_t x = r.x; x < r.x + r.w && x < width; x++) {
			pattern_store(base + (size_t)y * pitch_bytes + (size_t)x * bp, bp,
				      pattern_pixel(x, y, width, fmt));
		}
	}
	return 0;
}

static inline int pattern_fill(void *buf, uint32_t width, uint32_t height,
			       uint32_t pitch_bytes, uint32_t bpp, enum pattern_fmt fmt)
{
	struct pattern_rect all = { 0, 0, width, height };

	return pattern_fill_rect(buf, width, height, pitch_bytes, bpp, fmt, all);
}

/*
 * Read back rectangle r and compare with the pattern. step 1 checks every
 * pixel; step N > 1 only those whose offset from r's corner is a multiple of N
 * in x and y (a sparse grid, for a large uncached buffer). Fills *rep and
 * returns the mismatch count, or -1 for bad arguments.
 */
static inline int pattern_check_rect(const void *buf, uint32_t width, uint32_t height,
				     uint32_t pitch_bytes, uint32_t bpp,
				     enum pattern_fmt fmt, struct pattern_rect r,
				     uint32_t step, struct pattern_report *rep)
{
	uint32_t bp = pattern_bytes_per_pixel(fmt);
	const volatile uint8_t *base = (const volatile uint8_t *)buf;
	uint32_t sum = 2166136261u;

	if (!buf || !rep || bpp != bp * 8u || pitch_bytes < width * bp) {
		return -1;
	}
	if (step == 0u) {
		step = 1u;
	}
	rep->checked = 0;
	rep->mismatches = 0;
	rep->first_x = rep->first_y = rep->expected = rep->got = 0;
	for (uint32_t y = r.y; y < r.y + r.h && y < height; y += step) {
		for (uint32_t x = r.x; x < r.x + r.w && x < width; x += step) {
			uint32_t got = pattern_load(
				base + (size_t)y * pitch_bytes + (size_t)x * bp, bp);
			uint32_t exp = pattern_pixel(x, y, width, fmt);

			for (unsigned int i = 0; i < 4u; i++) {
				sum = (sum ^ ((got >> (8u * i)) & 0xffu)) * 16777619u;
			}
			rep->checked++;
			if (got != exp) {
				if (rep->mismatches == 0u) {
					rep->first_x = x;
					rep->first_y = y;
					rep->expected = exp;
					rep->got = got;
				}
				rep->mismatches++;
			}
		}
	}
	rep->checksum = sum;
	return (int)rep->mismatches;
}

static inline int pattern_check(const void *buf, uint32_t width, uint32_t height,
				uint32_t pitch_bytes, uint32_t bpp, enum pattern_fmt fmt,
				uint32_t step, struct pattern_report *rep)
{
	struct pattern_rect all = { 0, 0, width, height };

	return pattern_check_rect(buf, width, height, pitch_bytes, bpp, fmt, all, step, rep);
}

/*
 * The Pi 4's split: the console in the top half (board_rpi4.c prints to the
 * top half and the V3D clears the bottom half), here with the text band a
 * whole number of fbcon's 16-row glyph cells so no text row is cut. The
 * pattern band is the rest. Together they cover the screen, no overlap.
 */
static inline struct pattern_layout split_layout(uint32_t width, uint32_t height)
{
	uint32_t top = (height / 2u) & ~15u;
	struct pattern_layout l = {
		.text = { 0, 0, width, top },
		.pattern = { 0, top, width, height - top },
	};

	return l;
}

#endif /* RPI5_SCANOUT_PATTERN_H */
