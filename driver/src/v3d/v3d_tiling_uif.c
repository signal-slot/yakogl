/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A UIF store walker for 4-byte texels (docs/notes/v3d-tiling-path.md, "What
 * a fix could be", 1): the texels Mesa's v3d_store_tiled_image would write,
 * with the addresses stepped instead of computed per utile.
 *
 * Mesa's v3d_move_pixels_general_percpp calls v3d_get_uif_pixel_offset (two
 * ffs, a divide, an align and a multiply) for every utile. For cpp 4 a utile
 * is 4x4 texels, 64 B in rows of 16 B; a UIF block is 2x2 utiles, 8x8 texels
 * and 256 B contiguous (top left, top right, bottom left, bottom right); the
 * blocks are laid out in columns 4 blocks wide, each column mb_h block rows
 * of 1 KiB, mb_h being the padded height in blocks:
 *
 *   block (bx, by) = (bx / 4) * mb_h * 1024 + by' * 1024 + (bx % 4) * 256
 *
 * with by' = by ^ 16 in an odd column of a UIF_XOR level, by otherwise
 * (Mesa's mb_id, (mb_x / 4) * ((mb_h - 1) * 4) + mb_x + mb_y * 4, is the
 * same sum). Here a block row's offset is computed once for the even and once
 * for the odd columns, and the walk steps 256 B to the next block and one
 * column's bytes at every fourth.
 *
 * The utiles are stored by Mesa's v3d_store_utile (v3d_cpu_tiling.h): the
 * st1 code on aarch64, as Mesa's walker; the memcpy rows elsewhere.
 *
 * Only a box whose x, y, width and height are multiples of 8 (whole UIF
 * blocks) of a UIF level with cpp 4 is taken; v3d_tex_store_box hands every
 * other box to Mesa's function. tools/v3d/tiling_bench.sh compares the bytes
 * with Mesa's.
 *
 * Not a translation unit of its own: v3d_tex.c includes this file, so every
 * build that compiles v3d_tex.c (the CMake and Zephyr builds, the host tests
 * and tools that list v3d_tex.c by name) compiles the walker with it.
 */
#ifndef V3D_TILING_UIF_C
#define V3D_TILING_UIF_C

#include <stddef.h>
#include <string.h>

#include "v3d_tex.h"

/* Mesa's utile store and what it needs: DETECT_ARCH_AARCH64 selects the st1
 * body; the memcpy body adds to a void pointer. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wpointer-arith"
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"
#include "util/detect_arch.h"
#include "common/v3d_cpu_tiling.h"
#pragma GCC diagnostic pop

/* cpp 4: a utile row is 16 B, a utile 64 B, a block 256 B, a block row of a
 * 4-block column 1 KiB */
#define UIF4_UTILE_ROW 16u
#define UIF4_UTILE 64u
#define UIF4_BLOCK 256u
#define UIF4_BLOCK_ROW (4u * UIF4_BLOCK)
#define UIF4_BLOCK_TEXELS 8u

/* One UIF block: the 8 source rows at `src` into 256 B at `dst`. */
static inline void uif4_store_block(uint8_t *dst, const uint8_t *src, uint32_t src_stride)
{
	uint8_t *s = (uint8_t *)(uintptr_t)src; /* only read */
	uint8_t *s4 = s + (size_t)4 * src_stride;
	v3d_store_utile(dst, UIF4_UTILE_ROW, s, src_stride);
	v3d_store_utile(dst + UIF4_UTILE, UIF4_UTILE_ROW, s + UIF4_UTILE_ROW, src_stride);
	v3d_store_utile(dst + 2 * UIF4_UTILE, UIF4_UTILE_ROW, s4, src_stride);
	v3d_store_utile(dst + 3 * UIF4_UTILE, UIF4_UTILE_ROW, s4 + UIF4_UTILE_ROW, src_stride);
}

bool v3d_tex_store_uif4(uint8_t *level_base, const struct v3d_tex_slice *s, uint32_t cpp,
			uint32_t x, uint32_t y, uint32_t w, uint32_t h, const uint8_t *src,
			uint32_t src_stride)
{
	if (!level_base || !s || !src || cpp != 4 ||
	    (s->tiling != V3D_TEX_TILING_UIF_NO_XOR && s->tiling != V3D_TEX_TILING_UIF_XOR) ||
	    ((x | y | w | h | s->padded_height) & (UIF4_BLOCK_TEXELS - 1)) != 0 ||
	    src_stride < w * cpp)
		return false;

	const bool do_xor = s->tiling == V3D_TEX_TILING_UIF_XOR;
	const size_t column_bytes = (size_t)(s->padded_height / UIF4_BLOCK_TEXELS) * UIF4_BLOCK_ROW;
	const uint32_t bx0 = x / UIF4_BLOCK_TEXELS, bx1 = (x + w) / UIF4_BLOCK_TEXELS;
	const uint32_t by0 = y / UIF4_BLOCK_TEXELS, by1 = (y + h) / UIF4_BLOCK_TEXELS;
	uint8_t *const first_column = level_base + (size_t)(bx0 / 4) * column_bytes;

	for (uint32_t by = by0; by < by1; by++) {
		const uint8_t *src_block =
			src + (size_t)(by - by0) * UIF4_BLOCK_TEXELS * src_stride;
		/* the block row's offset in an even and in an odd column */
		const size_t row_even = (size_t)by * UIF4_BLOCK_ROW;
		const size_t row_odd = (size_t)(do_xor ? by ^ 16u : by) * UIF4_BLOCK_ROW;
		uint32_t column = bx0 / 4;
		uint8_t *column_base = first_column;
		uint8_t *dst = column_base + ((column & 1) ? row_odd : row_even) +
			       (size_t)(bx0 & 3) * UIF4_BLOCK;

		for (uint32_t bx = bx0; bx < bx1; bx++) {
			if ((bx & 3) == 0 && bx != bx0) {
				column++;
				column_base += column_bytes;
				dst = column_base + ((column & 1) ? row_odd : row_even);
			}
			uif4_store_block(dst, src_block, src_stride);
			dst += UIF4_BLOCK;
			src_block += UIF4_BLOCK_TEXELS * 4;
		}
	}
	return true;
}

#endif
