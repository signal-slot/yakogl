/* SPDX-License-Identifier: MIT */
/*
 * The byte offset of texel (x, y) inside a tiled V3D 4.2 level, from the
 * level's own start; image_h is the level's padded height. A checker's
 * reference, for tools/v3d/tfu_case.h and tex_tiling_bench.h, which look
 * up every texel of an image the driver (Mesa's v3d_store_tiled_image) or
 * the TFU wrote. The driver itself no longer has it: it tiles through
 * Mesa's own v3d_tiling.c (driver/src/v3d/v3d_tiling_mesa.c).
 *
 * Transcribed from Mesa 26.2.1's broadcom/common/v3d_tiling.c
 * (v3d_get_utile_pixel_offset, v3d_get_lt_pixel_offset,
 * v3d_get_ublinear_pixel_offset, v3d_get_uif_pixel_offset), which keeps
 * them static:
 *
 *   Copyright (c) 2014-2017 Broadcom
 *
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, sublicense,
 * and/or sell copies of the Software, and to permit persons to whom the
 * Software is furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice (including the next
 * paragraph) shall be included in all copies or substantial portions of the
 * Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE.
 */
#ifndef TEX_PIXEL_OFFSET_H
#define TEX_PIXEL_OFFSET_H

#include <stdbool.h>
#include <stdint.h>

#include "v3d_tex.h" /* enum v3d_tex_tiling */

static inline uint32_t tpo_utile_width(uint32_t cpp)
{
	return cpp <= 2 ? 8 : cpp <= 8 ? 4 : 2;
}

static inline uint32_t tpo_utile_height(uint32_t cpp)
{
	return cpp == 1 ? 8 : cpp <= 4 ? 4 : 2;
}

static inline uint32_t tpo_utile_pixel_offset(uint32_t cpp, uint32_t x, uint32_t y)
{
	return x * cpp + y * tpo_utile_width(cpp) * cpp;
}

static inline uint32_t tpo_log2(uint32_t v)
{
	uint32_t n = 0;
	while (v > 1) {
		v >>= 1;
		n++;
	}
	return n;
}

static inline uint32_t tpo_uif(uint32_t cpp, uint32_t image_h, uint32_t x, uint32_t y, bool do_xor)
{
	uint32_t utile_w = tpo_utile_width(cpp), utile_h = tpo_utile_height(cpp);
	uint32_t log2_mb_width = tpo_log2(utile_w * 2), log2_mb_height = tpo_log2(utile_h * 2);
	uint32_t mb_x = x >> log2_mb_width, mb_y = y >> log2_mb_height;
	uint32_t mb_pixel_x = x - (mb_x << log2_mb_width);
	uint32_t mb_pixel_y = y - (mb_y << log2_mb_height);

	if (do_xor && (mb_x / 4) & 1)
		mb_y ^= 0x10;

	uint32_t mb_h = ((image_h + (1u << log2_mb_height) - 1) & ~((1u << log2_mb_height) - 1)) >>
			log2_mb_height;
	uint32_t mb_id = ((mb_x / 4) * ((mb_h - 1) * 4)) + mb_x + mb_y * 4;
	bool top = mb_pixel_y < utile_h, left = mb_pixel_x < utile_w;
	return mb_id * 256 + (!top * 128 + !left * 64) +
	       tpo_utile_pixel_offset(cpp, mb_pixel_x & (utile_w - 1), mb_pixel_y & (utile_h - 1));
}

static inline uint32_t v3d_tex_pixel_offset(enum v3d_tex_tiling tiling, uint32_t cpp,
					    uint32_t image_h, uint32_t x, uint32_t y)
{
	uint32_t utile_w = tpo_utile_width(cpp), utile_h = tpo_utile_height(cpp);
	uint32_t in_utile = tpo_utile_pixel_offset(cpp, x & (utile_w - 1), y & (utile_h - 1));
	switch (tiling) {
	case V3D_TEX_TILING_UIF_XOR:
		return tpo_uif(cpp, image_h, x, y, true);
	case V3D_TEX_TILING_UIF_NO_XOR:
		return tpo_uif(cpp, image_h, x, y, false);
	case V3D_TEX_TILING_UBLINEAR_2_COLUMN:
	case V3D_TEX_TILING_UBLINEAR_1_COLUMN: {
		uint32_t number = tiling == V3D_TEX_TILING_UBLINEAR_2_COLUMN ? 2 : 1;
		uint32_t ub_x = x / (utile_w * 2), ub_y = y / (utile_h * 2);
		return 256 * (ub_y * number + ub_x) + ((x & utile_w) ? 64 : 0) +
		       ((y & utile_h) ? 128 : 0) + in_utile;
	}
	case V3D_TEX_TILING_LINEARTILE:
	default:
		return 64 * (x / utile_w + y / utile_h) + in_utile;
	}
}

#endif
