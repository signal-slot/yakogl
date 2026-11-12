/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pattern.h -- the mp2_display probe's colour bars. No Zephyr include: the
 * host test (test/pattern_test.sh) compiles pattern.c with the host cc.
 */
#ifndef MP2_DISPLAY_PATTERN_H
#define MP2_DISPLAY_PATTERN_H

#include <stddef.h>
#include <stdint.h>

#define PATTERN_BARS 8

enum pattern_format {
	PATTERN_ARGB8888, /* 32-bit word 0xAARRGGBB, little endian, alpha 0xff */
	PATTERN_RGB888,   /* bytes B, G, R */
	PATTERN_RGB565,   /* 16-bit word RRRRRGGGGGGBBBBB */
};

/* Bytes per pixel of a format. */
unsigned int pattern_bpp(enum pattern_format fmt);

/* The RGB of bar i (0..7): white, yellow, cyan, green, magenta, red, blue,
 * black, the order tools/probes/stm32mp257f_dk/display_check/colorbars.py
 * judges.
 */
void pattern_bar_rgb(unsigned int i, uint8_t rgb[3]);

/* The bar column x of a width-w picture is in: x * 8 / w. */
unsigned int pattern_bar_of(unsigned int x, unsigned int w);

/* Fill height lines of a linear buffer, stride bytes apart, with the 8
 * vertical bars. The padding after each line (stride - width * bpp) is
 * zeroed. Returns 0, or -1 for a stride too small or a zero size.
 */
int pattern_fill(void *buf, unsigned int width, unsigned int height, size_t stride,
		 enum pattern_format fmt);

#endif
