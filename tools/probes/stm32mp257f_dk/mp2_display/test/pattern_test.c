/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host helper of pattern_test.sh: fills a buffer with the bars and writes it
 * as a binary PPM (P6) on stdout, after checking the stride padding.
 *   pattern_test W H STRIDE argb8888|rgb888|rgb565 > shot.ppm
 */
#include "../src/pattern.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
	if (argc != 5) {
		return 2;
	}
	unsigned int w = atoi(argv[1]), h = atoi(argv[2]);
	size_t stride = atoi(argv[3]);
	enum pattern_format fmt = !strcmp(argv[4], "argb8888") ? PATTERN_ARGB8888
				  : !strcmp(argv[4], "rgb888")  ? PATTERN_RGB888
								: PATTERN_RGB565;
	unsigned int bpp = pattern_bpp(fmt);
	uint8_t *buf = malloc(stride * h);

	memset(buf, 0xa5, stride * h);
	if (pattern_fill(buf, w, h, stride, fmt) != 0) {
		return 1;
	}
	printf("P6\n%u %u\n255\n", w, h);
	for (unsigned int y = 0; y < h; y++) {
		const uint8_t *l = buf + y * stride;

		for (size_t i = (size_t)w * bpp; i < stride; i++) {
			if (l[i] != 0) {
				fprintf(stderr, "padding not zero at line %u\n", y);
				return 1;
			}
		}
		for (unsigned int x = 0; x < w; x++) {
			const uint8_t *p = l + (size_t)x * bpp;
			uint8_t r, g, b;

			if (fmt == PATTERN_ARGB8888) {
				if (p[3] != 0xff) {
					return 1;
				}
				r = p[2];
				g = p[1];
				b = p[0];
			} else if (fmt == PATTERN_RGB888) {
				r = p[2];
				g = p[1];
				b = p[0];
			} else {
				unsigned int v = p[0] | (p[1] << 8);

				r = ((v >> 11) & 31) * 255 / 31;
				g = ((v >> 5) & 63) * 255 / 63;
				b = (v & 31) * 255 / 31;
			}
			putchar(r);
			putchar(g);
			putchar(b);
		}
	}
	return 0;
}
