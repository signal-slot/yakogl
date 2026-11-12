/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include "pattern.h"

#include <string.h>

/* colorbars.py BARS: white, yellow, cyan, green, magenta, red, blue, black. */
static const uint8_t bars[PATTERN_BARS][3] = {
	{255, 255, 255}, {255, 255, 0}, {0, 255, 255}, {0, 255, 0},
	{255, 0, 255},   {255, 0, 0},   {0, 0, 255},   {0, 0, 0},
};

unsigned int pattern_bpp(enum pattern_format fmt)
{
	switch (fmt) {
	case PATTERN_ARGB8888:
		return 4;
	case PATTERN_RGB888:
		return 3;
	default:
		return 2;
	}
}

void pattern_bar_rgb(unsigned int i, uint8_t rgb[3])
{
	memcpy(rgb, bars[i % PATTERN_BARS], 3);
}

unsigned int pattern_bar_of(unsigned int x, unsigned int w)
{
	return (unsigned int)(((uint64_t)x * PATTERN_BARS) / w);
}

static void put(uint8_t *p, enum pattern_format fmt, const uint8_t *c)
{
	uint32_t v;

	switch (fmt) {
	case PATTERN_ARGB8888:
		p[0] = c[2];
		p[1] = c[1];
		p[2] = c[0];
		p[3] = 0xff;
		break;
	case PATTERN_RGB888:
		p[0] = c[2];
		p[1] = c[1];
		p[2] = c[0];
		break;
	default:
		v = ((uint32_t)(c[0] >> 3) << 11) | ((uint32_t)(c[1] >> 2) << 5) | (c[2] >> 3);
		p[0] = v & 0xff;
		p[1] = (v >> 8) & 0xff;
		break;
	}
}

int pattern_fill(void *buf, unsigned int width, unsigned int height, size_t stride,
		 enum pattern_format fmt)
{
	unsigned int bpp = pattern_bpp(fmt);
	uint8_t *line = buf;

	if (buf == NULL || width == 0 || height == 0 || stride < (size_t)width * bpp) {
		return -1;
	}
	/* Line 0 is built once and copied: the bars do not vary by row. */
	for (unsigned int x = 0; x < width; x++) {
		put(line + (size_t)x * bpp, fmt, bars[pattern_bar_of(x, width)]);
	}
	memset(line + (size_t)width * bpp, 0, stride - (size_t)width * bpp);
	for (unsigned int y = 1; y < height; y++) {
		memcpy(line + (size_t)y * stride, line, stride);
	}
	return 0;
}
