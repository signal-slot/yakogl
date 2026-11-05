/* SPDX-License-Identifier: MIT */
/*
 * V3D 4.2 textures (v3d_tex.h): the level layout, the texel tiling and the
 * TMU's texture and sampler state records, transcribed from Mesa 26.2.1.
 *
 *   Copyright (c) 2014-2017 Broadcom
 *   Copyright (C) 2012 Rob Clark <robclark@freedesktop.org>
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
#include "v3d_mesa_posix.h" /* first: see the file */
#include "v3d_tex.h" /* and through it v3d_cl.h: Mesa's packers */

#include <string.h>

/* Mesa's texel tiling (v3d_tiling_mesa.c) and its struct pipe_box; Mesa's
 * headers are not ours to keep free of the warnings our builds turn on. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"
#include "util/box.h"
#include "common/v3d_tiling.h"
#pragma GCC diagnostic pop

/* v3d_screen.h: "tunable parameters in the HW design, but all the V3D
 * implementations agree" */
#define V3D_UIFCFG_BANKS 8
#define V3D_UIFCFG_PAGE_SIZE 4096
#define V3D_PAGE_CACHE_SIZE (V3D_UIFCFG_PAGE_SIZE * V3D_UIFCFG_BANKS)
#define V3D_UBLOCK_SIZE 64
#define V3D_UIFBLOCK_SIZE (4 * V3D_UBLOCK_SIZE)
#define V3D_UIFBLOCK_ROW_SIZE (4 * V3D_UIFBLOCK_SIZE)

/* v3d_resource.c */
#define PAGE_UB_ROWS (V3D_UIFCFG_PAGE_SIZE / V3D_UIFBLOCK_ROW_SIZE)
#define PAGE_UB_ROWS_TIMES_1_5 ((PAGE_UB_ROWS * 3) >> 1)
#define PAGE_CACHE_UB_ROWS (V3D_PAGE_CACHE_SIZE / V3D_UIFBLOCK_ROW_SIZE)
#define PAGE_CACHE_MINUS_1_5_UB_ROWS (PAGE_CACHE_UB_ROWS - PAGE_UB_ROWS_TIMES_1_5)

static uint32_t align_u32(uint32_t v, uint32_t a)
{
	return (v + a - 1) & ~(a - 1);
}

/* util's u_minify */
static uint32_t minify(uint32_t v, uint32_t level)
{
	uint32_t m = v >> level;
	return m ? m : 1;
}

/* util_next_power_of_two */
static uint32_t next_pot(uint32_t x)
{
	uint32_t p = 1;
	while (p < x)
		p <<= 1;
	return p;
}

/* Mesa's v3d_tiling.c (v3d_tiling_mesa.c) for the utile size; every caller
 * has checked cpp against 1, 2, 4, 8 and 16, the values it takes. */
static uint32_t utile_width(uint32_t cpp)
{
	return v3d_utile_width((int)cpp);
}

static uint32_t utile_height(uint32_t cpp)
{
	return v3d_utile_height((int)cpp);
}

/* v3d_resource.c v3d_get_ub_pad */
static uint32_t get_ub_pad(uint32_t cpp, uint32_t height)
{
	uint32_t uif_block_h = utile_height(cpp) * 2;
	uint32_t height_ub = height / uif_block_h;
	uint32_t height_offset_in_pc = height_ub % PAGE_CACHE_UB_ROWS;

	/* For the perfectly-aligned-for-UIF-XOR case, don't add any pad. */
	if (height_offset_in_pc == 0)
		return 0;

	/* Try padding up to where we're offset by at least half a page. */
	if (height_offset_in_pc < PAGE_UB_ROWS_TIMES_1_5) {
		/* If we fit entirely in the page cache, don't pad. */
		if (height_ub < PAGE_CACHE_UB_ROWS)
			return 0;
		else
			return PAGE_UB_ROWS_TIMES_1_5 - height_offset_in_pc;
	}

	/* If we're close to being aligned to page cache size, then round up
	 * and rely on XOR. */
	if (height_offset_in_pc > PAGE_CACHE_MINUS_1_5_UB_ROWS)
		return PAGE_CACHE_UB_ROWS - height_offset_in_pc;

	/* Otherwise, we're far enough away (top and bottom) to not need any
	 * padding. */
	return 0;
}

/* v3d_resource.c v3d_get_dimension_mpad (:573-581), level >= 1: level 1's
 * size in blocks of `block` texels, rounded up to a power of two, back in
 * texels. "Through testing it was found that the block dimension should be
 * used" (its comment): an ETC2 width of 36 has 18 texels, 5 blocks at level
 * 1, so 8 blocks, 32 texels. With a block of 1 it is next_pow2(dimension >>
 * 1) minified. */
static uint32_t dimension_mpad_block(uint32_t dimension, uint32_t level, uint32_t block)
{
	uint32_t pot_dim = next_pot((minify(dimension, 1) + block - 1) / block);
	return minify(block * pot_dim, level - 1);
}

/* ... block size 1 */
static uint32_t dimension_mpad(uint32_t dimension, uint32_t level)
{
	return dimension_mpad_block(dimension, level, 1);
}

/* v3d_setup_slices' level_depth (v3d_resource.c:609-611, :636-639): level 0
 * has the depth itself (u_minify(depth, 0)); a level i >= 1 has
 * u_minify(pot_depth, i) with pot_depth = 2 * v3d_get_dimension_mpad(depth,
 * 1, 1) = 2 * next_pow2(u_minify(depth, 1)), the same power-of-two rule the
 * sides follow from level 2, one level earlier. So a depth of 6 has 4 slices
 * at level 1 (next_pow2(3)), a depth of 5 has 2, a depth of 9 has 4, and a
 * depth of 1 has 1 at every level. */
uint32_t v3d_tex_level_depth(uint32_t depth, uint32_t level)
{
	if (level == 0)
		return depth;
	return minify(2 * dimension_mpad(depth, 1), level);
}

/* v3d_resource.c v3d_setup_slices, for a never-shared resource (no winsys
 * stride) of `depth` slices (1: a 2D texture, a cube face or an array layer;
 * more: a 3D texture) whose format has blocks of block_w x block_h texels of
 * `cpp` bytes (1 x 1 and the texel's bytes: uncompressed; 4 x 4 and 8 or 16:
 * ETC2/EAC); `uif_top` as Mesa's: level 0 is UIF whatever its size. */
static bool layout(uint32_t width, uint32_t height, uint32_t depth, uint32_t levels, uint32_t cpp,
		   uint32_t block_w, uint32_t block_h, bool uif_top, struct v3d_tex_layout *l);

/* ... one slice a level, single-sampled, uncompressed: uif_top false */
bool v3d_tex_layout(uint32_t width, uint32_t height, uint32_t levels, uint32_t cpp,
		    struct v3d_tex_layout *l)
{
	return layout(width, height, 1, levels, cpp, 1, 1, false, l);
}

/* ... a compressed 2D image: Mesa's rsc->cpp is util_format_get_blocksize
 * (v3d_resource.c:797), the block is one texel of the tiling */
bool v3d_tex_layout_compressed(uint32_t width, uint32_t height, uint32_t levels, uint32_t block_w,
			       uint32_t block_h, uint32_t block_bytes, struct v3d_tex_layout *l)
{
	if (block_w == 0 || block_w > V3D_TEX_MAX_BLOCK_DIM || block_h == 0 ||
	    block_h > V3D_TEX_MAX_BLOCK_DIM || (block_bytes != 8 && block_bytes != 16))
		return false;
	return layout(width, height, 1, levels, block_bytes, block_w, block_h, false, l);
}

/* ... a 3D texture: `depth` slices at level 0, Mesa's level_depth below it */
bool v3d_tex_layout_3d(uint32_t width, uint32_t height, uint32_t depth, uint32_t levels,
		       uint32_t cpp, struct v3d_tex_layout *l)
{
	if (depth == 0 || depth >= (1u << 14))
		return false;
	return layout(width, height, depth, levels, cpp, 1, 1, false, l);
}

/* A multisampled (4x) width x height resource: one level of twice the width
 * and height, UIF at the top (v3d_resource.c:612-642: "MSAA
 * textures/renderbuffers are always laid out as single-level UIF",
 * `uif_top |= msaa`, `level_width *= 2; level_height *= 2`). */
bool v3d_tex_layout_msaa(uint32_t width, uint32_t height, uint32_t cpp, struct v3d_tex_layout *l)
{
	if (width == 0 || height == 0 || width >= (1u << 13) || height >= (1u << 13))
		return false;
	return layout(2 * width, 2 * height, 1, 1, cpp, 1, 1, true, l);
}

static bool layout(uint32_t width, uint32_t height, uint32_t depth, uint32_t levels, uint32_t cpp,
		   uint32_t block_w, uint32_t block_h, bool uif_top, struct v3d_tex_layout *l)
{
	if (!l || width == 0 || height == 0 || width >= (1u << 14) || height >= (1u << 14) ||
	    depth == 0 || levels == 0 || levels > V3D_TEX_MAX_LEVELS ||
	    (cpp != 1 && cpp != 2 && cpp != 4 && cpp != 8 && cpp != 16))
		return false;
	/* built apart and copied out at the end: a 3D stack past 32 bits is
	 * refused with `l` untouched */
	struct v3d_tex_layout out;
	memset(&out, 0, sizeof(out));
	out.width = width;
	out.height = height;
	out.levels = levels;
	out.cpp = cpp;

	uint64_t offset = 0; /* the slices' total, checked against 32 bits below */
	uint32_t utile_w = utile_width(cpp);
	uint32_t utile_h = utile_height(cpp);
	uint32_t uif_block_w = utile_w * 2;
	uint32_t uif_block_h = utile_h * 2;

	/* "power-of-two padding is based on level 1 ... Additionally the pot
	 * padding is based on the block size" (:596-608) */
	uint32_t pot_width = 2 * dimension_mpad_block(width, 1, block_w);
	uint32_t pot_height = 2 * dimension_mpad_block(height, 1, block_h);

	for (int i = (int)levels - 1; i >= 0; i--) {
		struct v3d_tex_slice *slice = &out.slices[i];
		uint32_t level_width, level_height;
		/* the slices of this level (:636-639): every one is tiled as
		 * the level's size says, the depth never enters the choice */
		uint32_t level_depth = v3d_tex_level_depth(depth, (uint32_t)i);

		if (i < 2) {
			level_width = minify(width, (uint32_t)i);
			level_height = minify(height, (uint32_t)i);
		} else {
			level_width = minify(pot_width, (uint32_t)i);
			level_height = minify(pot_height, (uint32_t)i);
		}
		/* from here on in blocks (:646-647): the tiling, the alignment
		 * and the stride take a block for a texel of `cpp` bytes */
		level_width = (level_width + block_w - 1) / block_w;
		level_height = (level_height + block_h - 1) / block_h;

		/* tiled; every level chosen by size, except level 0 of a
		 * uif_top resource, which is UIF (the (i != 0 || !uif_top)
		 * conditions of v3d_setup_slices) */
		bool by_size = i != 0 || !uif_top;
		if (by_size && (level_width <= utile_w || level_height <= utile_h)) {
			slice->tiling = V3D_TEX_TILING_LINEARTILE;
			level_width = align_u32(level_width, utile_w);
			level_height = align_u32(level_height, utile_h);
		} else if (by_size && level_width <= uif_block_w) {
			slice->tiling = V3D_TEX_TILING_UBLINEAR_1_COLUMN;
			level_width = align_u32(level_width, uif_block_w);
			level_height = align_u32(level_height, uif_block_h);
		} else if (by_size && level_width <= 2 * uif_block_w) {
			slice->tiling = V3D_TEX_TILING_UBLINEAR_2_COLUMN;
			level_width = align_u32(level_width, 2 * uif_block_w);
			level_height = align_u32(level_height, uif_block_h);
		} else {
			/* We align the width to a 4-block column of UIF blocks,
			 * but we only align height to UIF blocks. */
			level_width = align_u32(level_width, 4 * uif_block_w);
			level_height = align_u32(level_height, uif_block_h);

			slice->ub_pad = get_ub_pad(cpp, level_height);
			level_height += slice->ub_pad * uif_block_h;

			/* If the padding set us to to be aligned to the page
			 * cache size, then the HW will use the XOR bit on odd
			 * columns to get us perfectly misaligned. */
			if ((level_height / uif_block_h) %
			    (V3D_PAGE_CACHE_SIZE / V3D_UIFBLOCK_ROW_SIZE) == 0)
				slice->tiling = V3D_TEX_TILING_UIF_XOR;
			else
				slice->tiling = V3D_TEX_TILING_UIF_NO_XOR;
		}

		if (offset > UINT32_MAX)
			return false;
		slice->offset = (uint32_t)offset;
		slice->stride = level_width * cpp;
		slice->padded_height = level_height;
		slice->size = level_height * slice->stride;

		/* the level's slices one after the other (:722); `size` stays
		 * ONE slice's bytes, which is what the tiling and a 3D texture's
		 * slice stride (v3d_tex_slice_offset) take */
		uint64_t slice_total_size = (uint64_t)slice->size * level_depth;

		/* The HW aligns level 1's base to a page if any of level 1 or
		 * below could be UIF XOR.  The lower levels then inherit the
		 * alignment for as long as necessary, thanks to being power of
		 * two aligned. (The total of level 1's slices is what is
		 * aligned, :729-734, not each slice.) */
		if (i == 1 && level_width > 4 * uif_block_w &&
		    level_height > PAGE_CACHE_MINUS_1_5_UB_ROWS * uif_block_h)
			slice_total_size = (slice_total_size + V3D_UIFCFG_PAGE_SIZE - 1) &
					   ~(uint64_t)(V3D_UIFCFG_PAGE_SIZE - 1);

		offset += slice_total_size;
	}

	/* UIF/UBLINEAR levels need to be aligned to UIF-blocks, and LT only
	 * needs to be aligned to utile boundaries.  Since tiles are laid out
	 * from small to big in memory, we need to align the later UIF slices
	 * to UIF blocks, if they were preceded by non-UIF-block-aligned LT
	 * slices.
	 *
	 * We additionally align to 4k, which improves UIF XOR performance. */
	uint32_t page_align_offset = align_u32(out.slices[0].offset, 4096) - out.slices[0].offset;
	offset += page_align_offset;
	if (offset > UINT32_MAX)
		return false;
	out.size = (uint32_t)offset;
	for (uint32_t i = 0; i < levels; i++)
		out.slices[i].offset += page_align_offset;
	*l = out;
	return true;
}

/* ---- the texel tiling: Mesa's v3d_store_tiled_image ----------------------- */

/* The UIF walker for cpp 4, compiled with this file (see its comment) */
#include "v3d_tiling_uif.c"

/* The checks of every box: 0 refused, 1 an empty box (nothing to move), 2 a
 * box to move. The box must lie in the level's grid (gw x gh: texels for
 * the texel entry points, blocks for the block ones) and in the slice the
 * layout allocated: (x + w) * cpp within the stride and y + h within the
 * padded height. The second test holds for every grid the matching entry
 * point takes, and it is what keeps a layout of v3d_tex_layout_compressed
 * given to a texel entry point (its texel grid is up to block_w times wider
 * and block_h times taller than its slice) from moving texels past the
 * slice. Written so that no sum can
 * wrap: x + w and y + h are at most gw and gh, below 2^14. */
static int check_box(const struct v3d_tex_layout *l, uint32_t level, uint32_t gw, uint32_t gh,
		     uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint32_t row_stride)
{
	if (x > gw || w > gw - x || y > gh || h > gh - y)
		return 0;
	const struct v3d_tex_slice *s = &l->slices[level];
	if ((uint64_t)(x + w) * l->cpp > s->stride || y + h > s->padded_height)
		return 0;
	if (w == 0 || h == 0)
		return 1;
	if (row_stride < w * l->cpp)
		return 0;
	return 2;
}

/* The checked box to Mesa's v3d_store_tiled_image (broadcom/common/
 * v3d_tiling.c, compiled by v3d_tiling_mesa.c), with the level's start, its
 * padded height and the slice's tiling, whose enum is Mesa's enum
 * v3d_tiling_mode numbering (v3d_tex.h); a box of whole UIF blocks of a UIF
 * level with cpp 4 to the UIF walker. A block is one texel of l->cpp bytes,
 * as Mesa's own store moves "entire compressed blocks" (v3d_resource.c:
 * 320-321). */
static void store_checked(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level, uint32_t x,
			  uint32_t y, uint32_t w, uint32_t h, const uint8_t *src, uint32_t src_stride)
{
	const struct v3d_tex_slice *s = &l->slices[level];
	/* a UIF level with cpp 4 and a box of whole UIF blocks: the stepped
	 * walker (v3d_tiling_uif.c), the same bytes */
	if (v3d_tex_store_uif4(image + s->offset, s, l->cpp, x, y, w, h, src, src_stride))
		return;
	struct pipe_box box;
	memset(&box, 0, sizeof(box));
	box.x = (int32_t)x; /* all below 2^14: the layout's limit */
	box.y = (int32_t)y;
	box.width = (int32_t)w;
	box.height = (int32_t)h;
	box.depth = 1;
	/* the source is only read (v3d_move_tiled_image with is_load false) */
	v3d_store_tiled_image(image + s->offset, s->stride, (void *)(uintptr_t)src, src_stride,
			      (enum v3d_tiling_mode)s->tiling, (int)l->cpp, s->padded_height, &box);
}

/* ... and Mesa's v3d_load_tiled_image: the image is only read
 * (v3d_move_tiled_image with is_load true reads the tiled side), the raster
 * rows written. */
static void load_checked(const uint8_t *image, const struct v3d_tex_layout *l, uint32_t level, uint32_t x,
			 uint32_t y, uint32_t w, uint32_t h, uint8_t *dst, uint32_t dst_stride)
{
	const struct v3d_tex_slice *s = &l->slices[level];
	struct pipe_box box;
	memset(&box, 0, sizeof(box));
	box.x = (int32_t)x;
	box.y = (int32_t)y;
	box.width = (int32_t)w;
	box.height = (int32_t)h;
	box.depth = 1;
	v3d_load_tiled_image(dst, dst_stride, (void *)(uintptr_t)(image + s->offset), s->stride,
			     (enum v3d_tiling_mode)s->tiling, (int)l->cpp, s->padded_height, &box);
}

bool v3d_tex_store_box(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		       uint32_t x, uint32_t y, uint32_t w, uint32_t h, const uint8_t *src,
		       uint32_t src_stride)
{
	if (!image || !l || !src || level >= l->levels)
		return false;
	int c = check_box(l, level, minify(l->width, level), minify(l->height, level), x, y, w, h,
			  src_stride);
	if (c == 2)
		store_checked(image, l, level, x, y, w, h, src, src_stride);
	return c != 0;
}

bool v3d_tex_load_box(const uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		      uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint8_t *dst,
		      uint32_t dst_stride)
{
	if (!image || !l || !dst || level >= l->levels)
		return false;
	int c = check_box(l, level, minify(l->width, level), minify(l->height, level), x, y, w, h,
			  dst_stride);
	if (c == 2)
		load_checked(image, l, level, x, y, w, h, dst, dst_stride);
	return c != 0;
}

void v3d_tex_store_level(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			 const uint8_t *src, uint32_t src_stride)
{
	if (!l || level >= l->levels)
		return;
	(void)v3d_tex_store_box(image, l, level, 0, 0, minify(l->width, level),
				minify(l->height, level), src, src_stride);
}

/* ---- compressed images: whole blocks ---------------------------------------- */

/* The level's grid of blocks: the texel size of the level (u_minify of the
 * level 0 size) rounded up to blocks, as v3d_setup_slices counts a level
 * (:646-647) before its pot padding and alignment, which only add blocks
 * (the padding Mesa allocates past them is the caller's to leave alone). */
static bool block_grid(const struct v3d_tex_layout *l, uint32_t level, uint32_t block_w, uint32_t block_h,
		       uint32_t *gw, uint32_t *gh)
{
	if (!l || level >= l->levels || block_w == 0 || block_w > V3D_TEX_MAX_BLOCK_DIM || block_h == 0 ||
	    block_h > V3D_TEX_MAX_BLOCK_DIM)
		return false;
	*gw = (minify(l->width, level) + block_w - 1) / block_w;
	*gh = (minify(l->height, level) + block_h - 1) / block_h;
	return true;
}

bool v3d_tex_level_blocks(const struct v3d_tex_layout *l, uint32_t level, uint32_t block_w,
			  uint32_t block_h, uint32_t *blocks_w, uint32_t *blocks_h)
{
	uint32_t gw, gh;
	if (!blocks_w || !blocks_h || !block_grid(l, level, block_w, block_h, &gw, &gh))
		return false;
	*blocks_w = gw;
	*blocks_h = gh;
	return true;
}

bool v3d_tex_store_blocks(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			  uint32_t block_w, uint32_t block_h, uint32_t bx, uint32_t by, uint32_t bw,
			  uint32_t bh, const uint8_t *src, uint32_t src_stride)
{
	uint32_t gw, gh;
	if (!image || !src || !block_grid(l, level, block_w, block_h, &gw, &gh))
		return false;
	int c = check_box(l, level, gw, gh, bx, by, bw, bh, src_stride);
	if (c == 2)
		store_checked(image, l, level, bx, by, bw, bh, src, src_stride);
	return c != 0;
}

bool v3d_tex_load_blocks(const uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			 uint32_t block_w, uint32_t block_h, uint32_t bx, uint32_t by, uint32_t bw,
			 uint32_t bh, uint8_t *dst, uint32_t dst_stride)
{
	uint32_t gw, gh;
	if (!image || !dst || !block_grid(l, level, block_w, block_h, &gw, &gh))
		return false;
	int c = check_box(l, level, gw, gh, bx, by, bw, bh, dst_stride);
	if (c == 2)
		load_checked(image, l, level, bx, by, bw, bh, dst, dst_stride);
	return c != 0;
}

bool v3d_tex_store_level_blocks(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
				uint32_t block_w, uint32_t block_h, const uint8_t *src,
				uint32_t src_stride)
{
	uint32_t gw, gh;
	if (!block_grid(l, level, block_w, block_h, &gw, &gh))
		return false;
	return v3d_tex_store_blocks(image, l, level, block_w, block_h, 0, 0, gw, gh, src, src_stride);
}

/* ---- layered images: cube maps and arrays (v3d_resource.c) --------------- */

/* v3d_setup_slices' cube_map_stride for a non-3D texture (:757-764): from one
 * layer's mip tree to the next, 64-byte aligned. Level 0 is the last and
 * largest slice, so its end is the end of the tree. */
uint32_t v3d_tex_layer_stride(const struct v3d_tex_layout *l)
{
	if (!l || l->levels == 0)
		return 0;
	return align_u32(l->slices[0].offset + l->slices[0].size, 64);
}

/* rsc->size after the layers are added (:764): the first layer's layout and
 * a stride for each further layer */
bool v3d_tex_layers_size(const struct v3d_tex_layout *l, uint32_t layers, uint32_t *out)
{
	uint32_t stride = v3d_tex_layer_stride(l);
	if (!out || stride == 0 || layers == 0)
		return false;
	uint64_t size = (uint64_t)l->size + (uint64_t)stride * (layers - 1);
	if (size > UINT32_MAX)
		return false;
	*out = (uint32_t)size;
	return true;
}

/* v3d_layer_offset for a non-3D texture (:771-779) */
bool v3d_tex_layer_offset(const struct v3d_tex_layout *l, uint32_t level, uint32_t layer,
			  uint32_t layers, uint32_t *out)
{
	uint32_t stride = v3d_tex_layer_stride(l);
	if (!out || stride == 0 || level >= l->levels || layer >= layers)
		return false;
	uint64_t off = (uint64_t)l->slices[level].offset + (uint64_t)stride * layer;
	if (off > UINT32_MAX)
		return false;
	*out = (uint32_t)off;
	return true;
}

/* v3d_layer_offset for a 3D texture (:776-777): slice `slice` of a level is
 * `slice` slice sizes after the level; the level's slices are Mesa's
 * level_depth (v3d_tex_level_depth), which the layout allocated. */
bool v3d_tex_slice_offset(const struct v3d_tex_layout *l, uint32_t level, uint32_t slice,
			  uint32_t depth, uint32_t *out)
{
	if (!l || !out || depth == 0 || level >= l->levels || slice >= v3d_tex_level_depth(depth, level))
		return false;
	uint64_t off = (uint64_t)l->slices[level].offset + (uint64_t)l->slices[level].size * slice;
	if (off > UINT32_MAX)
		return false;
	*out = (uint32_t)off;
	return true;
}

/* ---- the source formats ---------------------------------------------------- */

/* Mesa's format table rows (v3dx_format_table.c, V3D 4.2):
 *   FORMAT(B8G8R8A8_UNORM, RGBA8, RGBA8, SWIZ_ZYXW, 16, 0)
 *   FORMAT(R8G8B8A8_UNORM, RGBA8, RGBA8, SWIZ_XYZW, 16, 0)
 *   FORMAT(A8_UNORM,       NO,    R8,    SWIZ_000X, 16, 0)
 *   FORMAT(L8_UNORM,       NO,    R8,    SWIZ_XXX1, 16, 0)
 *   FORMAT(L8A8_UNORM,     NO,    RG8,   SWIZ_XXXY, 16, 0)
 *   FORMAT(R32_FLOAT,      R32F,  R32F,  SWIZ_X001, 32, 1)   (v3dx_format_table.c:91)
 *   FORMAT(S8_UINT_Z24_UNORM, D24S8, DEPTH24_X8, SWIZ_XXXX, 32, 1)  (:149)
 * and for RGB565 and the GL core's ES 3.0 storage classes:
 *   FORMAT(R8G8B8A8_SNORM,    NO,         RGBA8_SNORM, SWIZ_XYZW, 16, 0)  (:69)
 *   FORMAT(R10G10B10A2_UNORM, RGB10_A2,   RGB10_A2,    SWIZ_XYZW, 16, 0)  (:71)
 *   FORMAT(R10G10B10A2_UINT,  RGB10_A2UI, RGB10_A2UI,  SWIZ_XYZW, 16, 0)  (:75)
 *   FORMAT(B5G6R5_UNORM,      BGR565,     RGB565,      SWIZ_XYZ1, 16, 0)  (:81)
 *   FORMAT(R16_FLOAT,         R16F,       R16F,        SWIZ_X001, 16, 0)  (:90)
 *   FORMAT(R16G16B16A16_FLOAT, RGBA16F,   RGBA16F,     SWIZ_XYZW, 16, 0)  (:100)
 *   FORMAT(R32G32B32A32_FLOAT, RGBA32F,   RGBA32F,     SWIZ_XYZW, 32, 4)  (:101)
 *   FORMAT(R8G8B8A8_SINT,     RGBA8I,     RGBA8I,      SWIZ_XYZW, 16, 0)  (:122)
 *   FORMAT(R8G8B8A8_UINT,     RGBA8UI,    RGBA8UI,     SWIZ_XYZW, 16, 0)  (:123)
 *   FORMAT(R16G16B16A16_SINT, RGBA16I,    RGBA16I,     SWIZ_XYZW, 16, 0)  (:129)
 *   FORMAT(R16G16B16A16_UINT, RGBA16UI,   RGBA16UI,    SWIZ_XYZW, 16, 0)  (:130)
 *   FORMAT(R32G32B32A32_SINT, RGBA32I,    RGBA32I,     SWIZ_XYZW, 32, 4)  (:136)
 *   FORMAT(R32G32B32A32_UINT, RGBA32UI,   RGBA32UI,    SWIZ_XYZW, 32, 4)  (:137)
 *   FORMAT(R11G11B10_FLOAT,   R11F_G11F_B10F, R11F_G11F_B10F, SWIZ_XYZ1, 16, 0)  (:146)
 *   FORMAT(R9G9B9E5_FLOAT,    NO,         RGB9_E5,     SWIZ_XYZ1, 16, 0)  (:147)
 * and the ES 3.0 compressed formats (:159-168), 4x4 blocks of 8 or 16 bytes
 * (util_format_get_blocksize), the sRGB rows with the linear row's type:
 *   FORMAT(ETC2_RGB8,       NO, RGB8_ETC2,                SWIZ_XYZ1, 16, 0)
 *   FORMAT(ETC2_SRGB8,      NO, RGB8_ETC2,                SWIZ_XYZ1, 16, 0)
 *   FORMAT(ETC2_RGB8A1,     NO, RGB8_PUNCHTHROUGH_ALPHA1, SWIZ_XYZW, 16, 0)
 *   FORMAT(ETC2_SRGB8A1,    NO, RGB8_PUNCHTHROUGH_ALPHA1, SWIZ_XYZW, 16, 0)
 *   FORMAT(ETC2_RGBA8,      NO, RGBA8_ETC2_EAC,           SWIZ_XYZW, 16, 0)
 *   FORMAT(ETC2_SRGBA8,     NO, RGBA8_ETC2_EAC,           SWIZ_XYZW, 16, 0)
 *   FORMAT(ETC2_R11_UNORM,  NO, R11_EAC,                  SWIZ_X001, 16, 0)
 *   FORMAT(ETC2_R11_SNORM,  NO, SIGNED_R11_EAC,           SWIZ_X001, 16, 0)
 *   FORMAT(ETC2_RG11_UNORM, NO, RG11_EAC,                 SWIZ_XY01, 16, 0)
 *   FORMAT(ETC2_RG11_SNORM, NO, SIGNED_RG11_EAC,          SWIZ_XY01, 16, 0)
 * (the line numbers of Mesa 26.2.1's file, third_party/mesa-26.2.1), with the
 * swizzle translated as v3d_translate_pipe_swizzle does (v3d_util.c:
 * X..W -> 2..5). A row with cpp 0 is a format this cut refuses. */
#define XYZW { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA }
#define XYZ1 { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ONE }
#define X001 { V3D_SWIZZLE_RED, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE }
#define XY01 { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE }
static const struct {
	const char *name;
	struct v3d_tex_format f;
	/* a compressed row: its block is block x block texels (0: 1 x 1,
	 * f.cpp the texel's bytes); srgb: util_format_is_srgb, the state's
	 * sRGB bit (v3dx_state.c:1023-1025) */
	uint8_t block;
	bool srgb;
} formats[V3D_TEX_FORMAT_COUNT] = {
	[V3D_TEX_FORMAT_RGBA8] = { "RGBA8",
				   { V3D_TEXTURE_DATA_FORMAT_RGBA8, 4,
				     { V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE,
				       V3D_SWIZZLE_ALPHA },
				     16 } },
	[V3D_TEX_FORMAT_BGRA8] = { "BGRA8",
				   { V3D_TEXTURE_DATA_FORMAT_RGBA8, 4,
				     { V3D_SWIZZLE_BLUE, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_RED,
				       V3D_SWIZZLE_ALPHA },
				     16 } },
	[V3D_TEX_FORMAT_RGB8] = { "RGB8", { 0 } },
	[V3D_TEX_FORMAT_L8] = { "L8",
				{ V3D_TEXTURE_DATA_FORMAT_R8, 1,
				  { V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_ONE },
				  16 } },
	[V3D_TEX_FORMAT_A8] = { "A8",
				{ V3D_TEXTURE_DATA_FORMAT_R8, 1,
				  { V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_RED },
				  16 } },
	[V3D_TEX_FORMAT_L8A8] = { "L8A8",
				  { V3D_TEXTURE_DATA_FORMAT_RG8, 2,
				    { V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN },
				    16 } },
	/* GL's UNSIGNED_SHORT_5_6_5: R in the high bits, B in the low ones
	 * (v3d_packet.xml: "B in low bits"); an EGLImage of an RGB565 buffer */
	[V3D_TEX_FORMAT_RGB565] = { "RGB565", { V3D_TEXTURE_DATA_FORMAT_RGB565, 2, XYZ1, 16 } },
	[V3D_TEX_FORMAT_RGBA4] = { "RGBA4", { 0 } },
	[V3D_TEX_FORMAT_RGB5_A1] = { "RGB5_A1", { 0 } },
	/* return size 32: a look-up of it must set TMU Config Parameter 1's
	 * "Output Type 32-bit" (v3d_packet.xml) and read a word per channel */
	[V3D_TEX_FORMAT_R32F] = { "R32F",
				  { V3D_TEXTURE_DATA_FORMAT_R32F, 4,
				    { V3D_SWIZZLE_RED, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE },
				    32 } },
	/* the depth/stencil buffer the jobs store (v3d_job.c: D24S8 in Mesa's
	 * zsbuf layout), sampled as its depth by the texture unit: the TMU
	 * reads bits 31..8 of each word as a 24-bit unorm depth, return size
	 * 32 as every depth format (v3dx_format_table.c:149-156). The backend
	 * puts the GL swizzle on the record (d, d, d, 1 or d, 0, 0, 1), as it
	 * does for R32F; Mesa's row is XXXX. */
	[V3D_TEX_FORMAT_D24S8] = { "D24S8",
				   { V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, 4,
				     { V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_RED, V3D_SWIZZLE_RED },
				     32 } },
	/* The ES 3.0 storage classes, four components each (the GL core
	 * stores a narrower format with (0, 0, 0, 1) in the components it
	 * lacks). The integer ones return raw integers, 16 bits a channel
	 * below 32-bit channels; the 32-bit ones (and RGBA32F) one 32-bit word
	 * a channel, return size 32 as R32F's. */
	[V3D_TEX_FORMAT_R16F] = { "R16F", { V3D_TEXTURE_DATA_FORMAT_R16F, 2, X001, 16 } },
	[V3D_TEX_FORMAT_RGBA16F] = { "RGBA16F", { V3D_TEXTURE_DATA_FORMAT_RGBA16F, 8, XYZW, 16 } },
	[V3D_TEX_FORMAT_R11G11B10F] = { "R11G11B10F",
					{ V3D_TEXTURE_DATA_FORMAT_R11F_G11F_B10F, 4, XYZ1, 16 } },
	[V3D_TEX_FORMAT_RGB9E5] = { "RGB9E5", { V3D_TEXTURE_DATA_FORMAT_RGB9_E5, 4, XYZ1, 16 } },
	[V3D_TEX_FORMAT_RGB10A2] = { "RGB10A2", { V3D_TEXTURE_DATA_FORMAT_RGB10_A2, 4, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGB10A2UI] = { "RGB10A2UI",
				       { V3D_TEXTURE_DATA_FORMAT_RGB10_A2UI, 4, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA8_SNORM] = { "RGBA8_SNORM",
					 { V3D_TEXTURE_DATA_FORMAT_RGBA8_SNORM, 4, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA8UI] = { "RGBA8UI", { V3D_TEXTURE_DATA_FORMAT_RGBA8UI, 4, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA8I] = { "RGBA8I", { V3D_TEXTURE_DATA_FORMAT_RGBA8I, 4, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA16UI] = { "RGBA16UI", { V3D_TEXTURE_DATA_FORMAT_RGBA16UI, 8, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA16I] = { "RGBA16I", { V3D_TEXTURE_DATA_FORMAT_RGBA16I, 8, XYZW, 16 } },
	[V3D_TEX_FORMAT_RGBA32UI] = { "RGBA32UI",
				      { V3D_TEXTURE_DATA_FORMAT_RGBA32UI, 16, XYZW, 32 } },
	[V3D_TEX_FORMAT_RGBA32I] = { "RGBA32I", { V3D_TEXTURE_DATA_FORMAT_RGBA32I, 16, XYZW, 32 } },
	[V3D_TEX_FORMAT_RGBA32F] = { "RGBA32F", { V3D_TEXTURE_DATA_FORMAT_RGBA32F, 16, XYZW, 32 } },
	/* ETC2 and EAC: cpp is the block's bytes, the tiling's texel
	 * (docs/notes/v3d-tex-layout-es3.md section 4) */
	[V3D_TEX_FORMAT_ETC2_RGB8] = { "ETC2_RGB8",
				       { V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, 8, XYZ1, 16 }, 4, false },
	[V3D_TEX_FORMAT_ETC2_SRGB8] = { "ETC2_SRGB8",
					{ V3D_TEXTURE_DATA_FORMAT_RGB8_ETC2, 8, XYZ1, 16 }, 4, true },
	[V3D_TEX_FORMAT_ETC2_RGB8A1] = { "ETC2_RGB8A1",
					 { V3D_TEXTURE_DATA_FORMAT_RGB8_PUNCHTHROUGH_ALPHA1, 8, XYZW, 16 },
					 4, false },
	[V3D_TEX_FORMAT_ETC2_SRGB8A1] = { "ETC2_SRGB8A1",
					  { V3D_TEXTURE_DATA_FORMAT_RGB8_PUNCHTHROUGH_ALPHA1, 8, XYZW, 16 },
					  4, true },
	[V3D_TEX_FORMAT_ETC2_RGBA8] = { "ETC2_RGBA8",
					{ V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, 16, XYZW, 16 }, 4, false },
	[V3D_TEX_FORMAT_ETC2_SRGBA8] = { "ETC2_SRGBA8",
					 { V3D_TEXTURE_DATA_FORMAT_RGBA8_ETC2_EAC, 16, XYZW, 16 }, 4, true },
	[V3D_TEX_FORMAT_ETC2_R11_UNORM] = { "ETC2_R11_UNORM",
					    { V3D_TEXTURE_DATA_FORMAT_R11_EAC, 8, X001, 16 }, 4, false },
	[V3D_TEX_FORMAT_ETC2_R11_SNORM] = { "ETC2_R11_SNORM",
					    { V3D_TEXTURE_DATA_FORMAT_SIGNED_R11_EAC, 8, X001, 16 }, 4, false },
	[V3D_TEX_FORMAT_ETC2_RG11_UNORM] = { "ETC2_RG11_UNORM",
					     { V3D_TEXTURE_DATA_FORMAT_RG11_EAC, 16, XY01, 16 }, 4, false },
	[V3D_TEX_FORMAT_ETC2_RG11_SNORM] = { "ETC2_RG11_SNORM",
					     { V3D_TEXTURE_DATA_FORMAT_SIGNED_RG11_EAC, 16, XY01, 16 }, 4,
					     false },
};
#undef XYZW
#undef XYZ1
#undef X001
#undef XY01

bool v3d_tex_format_get(enum v3d_tex_format_id id, struct v3d_tex_format *out)
{
	if ((unsigned)id >= V3D_TEX_FORMAT_COUNT || !out || formats[id].f.cpp == 0)
		return false;
	*out = formats[id].f;
	return true;
}

bool v3d_tex_format_block(enum v3d_tex_format_id id, uint32_t *block_w, uint32_t *block_h)
{
	if ((unsigned)id >= V3D_TEX_FORMAT_COUNT || !block_w || !block_h || formats[id].f.cpp == 0)
		return false;
	*block_w = *block_h = formats[id].block ? formats[id].block : 1;
	return true;
}

bool v3d_tex_format_is_srgb(enum v3d_tex_format_id id)
{
	return (unsigned)id < V3D_TEX_FORMAT_COUNT && formats[id].srgb;
}

const char *v3d_tex_format_name(enum v3d_tex_format_id id)
{
	if ((unsigned)id >= V3D_TEX_FORMAT_COUNT)
		return "unknown";
	return formats[id].name;
}

/* ---- the records ---------------------------------------------------------- */

/* llroundf: half away from zero, as util_bitpack_[us]fixed round */
static int64_t round_half_away(float v)
{
	return (int64_t)(v >= 0.0f ? v + 0.5f : v - 0.5f);
}

void v3d_tex_state_from_layout(struct v3d_tex_state *s, const struct v3d_tex_layout *l)
{
	const struct v3d_tex_slice *s0 = &l->slices[0];

	s->level_0_is_strictly_uif = s0->tiling == V3D_TEX_TILING_UIF_XOR ||
				     s0->tiling == V3D_TEX_TILING_UIF_NO_XOR;
	s->level_0_xor_enable = s0->tiling == V3D_TEX_TILING_UIF_XOR;
	s->level_0_ub_pad = 0;
	s->extended = false;
	if (s->level_0_is_strictly_uif) {
		s->level_0_ub_pad = s0->ub_pad;
		s->extended = true;
	}
	/* v3d_setup_slices' cube_map_stride for a non-3D texture, which
	 * v3d_setup_texture_shader_state programs whatever the layer count */
	s->array_stride_64 = align_u32(s0->offset + s0->size, 64) / 64;
}

/* The same fields for a 3D texture (v3d_tex_layout_3d): Mesa's
 * cube_map_stride is then level 0's slice size (v3d_resource.c:765-766, "we
 * need to program the stride between slices of miplevel 0"), and the state
 * takes it / 64 as for an array (v3dx_state.c:961). A slice is whole utiles
 * of 64 bytes, so the division is exact. */
void v3d_tex_state_from_layout_3d(struct v3d_tex_state *s, const struct v3d_tex_layout *l)
{
	v3d_tex_state_from_layout(s, l);
	s->array_stride_64 = l->slices[0].size / 64;
}

/* A type some supported format has: the TMU would read any other one as a
 * different format, so it is refused rather than packed. */
static bool type_is_supported(uint32_t type)
{
	for (unsigned i = 0; i < V3D_TEX_FORMAT_COUNT; i++)
		if (formats[i].f.cpp != 0 && formats[i].f.type == type)
			return true;
	return false;
}

bool v3d_tex_pack_state(uint8_t dst[V3D_TEX_STATE_BYTES], const struct v3d_tex_state *s)
{
	const uint32_t max14 = (1u << 14) - 1;
	if (!dst || !s || !type_is_supported(s->type) || (s->base & 63) != 0 ||
	    s->width == 0 || s->width > max14 || s->height == 0 || s->height > max14 ||
	    s->depth == 0 || s->depth > max14 || s->max_level > 15 ||
	    s->base_level > s->max_level || s->level_0_ub_pad > 15 ||
	    s->array_stride_64 >= (1u << 26))
		return false;
	for (int i = 0; i < 4; i++)
		if (s->swizzle[i] > V3D_SWIZZLE_ALPHA)
			return false;

	/* Mesa's packer (v3d_cl.h), into a copy that is stored with one
	 * memcpy: `dst` is in a job arena the CPU maps uncached. The base
	 * pointer is an address field above the flag bits 0..5: the image is
	 * at least 64-byte aligned. */
	struct V3D42_TEXTURE_SHADER_STATE t = { V3D42_TEXTURE_SHADER_STATE_header };
	t.texture_base_pointer = s->base;
	/* The sRGB bit (bit 3) is packed outside the checked round trip:
	 * Mesa's unpacker reads the base pointer as bits 0..31 whole, flag
	 * bits included, so a record with the bit set never unpacked to the
	 * values it was packed from and every sRGB texture was refused. The
	 * bit is ORed in below, where Mesa's packer puts it (cl[0] |=
	 * util_bitpack_uint(srgb, 3, 3)); the base's own bits 0..5 are zero
	 * (checked above). */
	t.array_stride_64_byte_aligned = s->array_stride_64;
	t.image_width = s->width;
	t.image_height = s->height;
	t.image_depth = s->depth;
	t.texture_type = s->type;
	t.extended = s->extended;
	t.swizzle_r = s->swizzle[0];
	t.swizzle_g = s->swizzle[1];
	t.swizzle_b = s->swizzle[2];
	t.swizzle_a = s->swizzle[3];
	t.max_level = s->max_level;
	t.base_level = s->base_level;
	t.level_0_ub_pad = s->level_0_ub_pad;
	t.level_0_xor_enable = s->level_0_xor_enable;
	t.level_0_is_strictly_uif = s->level_0_is_strictly_uif;
	uint8_t out[V3D42_TEXTURE_SHADER_STATE_length];
	if (!v3d_pack_TEXTURE_SHADER_STATE(out, &t))
		return false;
	if (s->srgb)
		out[0] |= 1u << 3;
	memcpy(dst, out, V3D_TEX_STATE_BYTES);
	return true;
}

static float clampf(float v, float lo, float hi)
{
	return v < lo ? lo : (v > hi ? hi : v);
}

bool v3d_tex_pack_sampler(uint8_t dst[V3D_SAMPLER_STATE_BYTES], const struct v3d_sampler *s)
{
	if (!dst || !s || s->wrap_s > V3D_WRAP_MODE_MIRROR || s->wrap_t > V3D_WRAP_MODE_MIRROR ||
	    s->wrap_r > V3D_WRAP_MODE_MIRROR || s->border_color_mode > 2 ||
	    s->min_lod != s->min_lod || s->max_lod != s->max_lod || s->lod_bias != s->lod_bias)
		return false;
	/* Fixed Bias is s8.8 in 16 bits (util_bitpack_sfixed); the first test
	 * keeps the conversion to an integer defined */
	if (s->lod_bias < -129.0f || s->lod_bias > 129.0f)
		return false;
	int64_t bias = round_half_away(s->lod_bias * 256.0f);
	if (bias < -32768 || bias > 32767)
		return false;

	/* v3d_upload_sampler_state_variant */
	float min_lod = clampf(s->min_lod, 0.0f, 15.0f);
	float max_lod = s->max_lod > s->min_lod ? s->max_lod : s->min_lod;
	max_lod = clampf(max_lod, 0.0f, 15.0f);
	/* "If we're not doing inter-miplevel filtering, we need to clamp the
	 * LOD so that we only sample from baselevel. However, we need to still
	 * allow the calculated LOD to be fractionally over the baselevel, so
	 * that the HW can decide between the min and mag filters." */
	if (s->mip_none) {
		if (min_lod > 1.0f / 256.0f)
			min_lod = 1.0f / 256.0f;
		if (max_lod > 1.0f / 256.0f)
			max_lod = 1.0f / 256.0f;
	}

	/* Mesa's packer (v3d_cl.h), into a copy stored with one memcpy (see
	 * v3d_tex_pack_state). The LODs are u4.8 and the bias s8.8 fields;
	 * they are given as the values the fields hold, rounded as above. */
	struct V3D42_SAMPLER_STATE p = { V3D42_SAMPLER_STATE_header };
	p.mag_filter_nearest = s->mag_nearest;
	p.min_filter_nearest = s->min_nearest;
	p.mip_filter_nearest = s->mip_nearest;
	/* anisotropy off; depth compare NEVER (0); sRGB decode on */
	p.min_level_of_detail = (float)round_half_away(min_lod * 256.0f) / 256.0f;
	p.max_level_of_detail = (float)round_half_away(max_lod * 256.0f) / 256.0f;
	p.fixed_bias = (float)bias / 256.0f;
	p.wrap_s = s->wrap_s;
	p.wrap_t = s->wrap_t;
	p.wrap_r = s->wrap_r;
	p.border_color_mode = s->border_color_mode;
	/* border colour words 0..3: 0 (no border in GL ES 2.0) */
	uint8_t out[V3D42_SAMPLER_STATE_length];
	if (!v3d_pack_SAMPLER_STATE(out, &p))
		return false;
	memcpy(dst, out, V3D_SAMPLER_STATE_BYTES);
	return true;
}

/* ---- the TMU configuration uniforms ---------------------------------------- */

bool v3d_tex_p0(uint32_t tex_state_addr, uint32_t data, uint32_t *out)
{
	uint32_t payload = data & 0x00ffffffu; /* v3d_unit_data_get_offset */
	if (!out || (tex_state_addr & (V3D_TEX_RECORD_ALIGN - 1)) != 0 || (payload & ~0xfu) != 0)
		return false;
	*out = tex_state_addr | payload;
	return true;
}

bool v3d_tex_p1(uint32_t sampler_addr, uint32_t data, uint32_t *out)
{
	uint32_t payload = data & 0x00ffffffu;
	if (!out || (sampler_addr & (V3D_TEX_RECORD_ALIGN - 1)) != 0 || (payload & ~0x7u) != 0)
		return false;
	*out = sampler_addr | payload;
	return true;
}

/* v3dx_tfu.c:120-176 (V3D 4.2): IOS = height << 16 | width; IIA = the
 * source; IIS = a raster source's stride in texels; ICFG = input format
 * raster (0) << 18 | data type << 9 | mip levels after the first (0) << 5,
 * and for a UIF destination OPAD << 22, the UIF block rows of padding
 * beyond those the height needs; IOA = destination | its format (3 + the
 * tiling's distance from LINEARTILE) << 3. */
bool v3d_tex_tfu_pack(const struct v3d_tex_layout *l, uint32_t ttype, uint32_t src_gpu, uint32_t src_stride,
		      uint32_t dst_gpu, struct v3d_tex_tfu_job *j)
{
	if (!l || !j || l->levels != 1 || l->cpp == 0 || src_stride % l->cpp != 0 ||
	    src_stride < l->width * l->cpp || l->width >= 16384u || l->height >= 16384u ||
	    (src_gpu & 3u) != 0)
		return false;
	const struct v3d_tex_slice *s = &l->slices[0];
	const uint32_t ioa = dst_gpu + s->offset;
	/* the TFU writes width x height texels of cpp bytes: a layout whose
	 * slice is smaller (a compressed one, counted in blocks) is refused */
	if ((ioa & 0x3fu) != 0 || (uint64_t)l->width * l->cpp > s->stride ||
	    l->height > s->padded_height)
		return false;
	struct v3d_tex_tfu_job out = { 0 };
	out.ios = l->height << 16 | l->width;
	out.iia = src_gpu;
	out.iis = src_stride / l->cpp;
	out.icfg = 0u << 18 | ttype << 9 | 0u << 5;
	out.ioa = ioa | (3u + ((uint32_t)s->tiling - V3D_TEX_TILING_LINEARTILE)) << 3;
	if (s->tiling == V3D_TEX_TILING_UIF_NO_XOR || s->tiling == V3D_TEX_TILING_UIF_XOR) {
		const uint32_t ubh = 2u * utile_height(l->cpp);
		const uint32_t implicit = (l->height + ubh - 1u) / ubh * ubh;
		out.icfg |= (s->padded_height - implicit) / ubh << 22;
	}
	*j = out;
	return true;
}

/* v3dx_tfu.c:97-163 (Mesa 26.2.1, V3D 4.2), the for_mipmap call of
 * v3d_generate_mipmap: one resource as source and destination, base level 0,
 * `levels - 1` levels after it. ICFG = the source's tiled format (11 + its
 * distance from LINEARTILE) << 18 | data type << 9 | levels after the first
 * (NUMMM) << 5, and for a UIF destination OPAD << 22, the UIF block rows of
 * padding beyond those the height needs (v3dx_tfu.c:154-162 sets it for
 * every UIF destination, DIMTW or not; its comment says the hardware infers
 * the tiling of levels 1+); IIS the UIF source's padded height in UIF
 * blocks; IOA = level 0 | the same format (3 + the distance) << 3 | DIMTW
 * (bit 0: level 0 is not written). */
bool v3d_tex_tfu_pack_mipmap(const struct v3d_tex_layout *l, uint32_t ttype, uint32_t image_gpu,
			     struct v3d_tex_tfu_job *j)
{
	uint32_t cpp;
	switch (ttype) {
	case V3D_TFU_TTYPE_R8:
		cpp = 1;
		break;
	case V3D_TFU_TTYPE_RG8:
	case V3D_TFU_TTYPE_R16F:
		cpp = 2;
		break;
	case V3D_TFU_TTYPE_RGBA8:
	case V3D_TFU_TTYPE_R32F:
		cpp = 4;
		break;
	default:
		return false;
	}
	if (!l || !j || l->levels < 2 || l->levels > V3D_TEX_MAX_LEVELS || l->cpp != cpp ||
	    l->width >= 16384u || l->height >= 16384u)
		return false;
	const struct v3d_tex_slice *s = &l->slices[0];
	const uint32_t level0 = image_gpu + s->offset;
	if ((level0 & 0x3fu) != 0 || s->tiling < V3D_TEX_TILING_LINEARTILE ||
	    (uint64_t)l->width * l->cpp > s->stride || l->height > s->padded_height)
		return false;
	const uint32_t distance = (uint32_t)s->tiling - V3D_TEX_TILING_LINEARTILE;
	struct v3d_tex_tfu_job out = { 0 };
	out.ios = l->height << 16 | l->width;
	out.iia = level0;
	out.icfg = (11u + distance) << 18 | ttype << 9 | (l->levels - 1u) << 5;
	if (s->tiling == V3D_TEX_TILING_UIF_NO_XOR || s->tiling == V3D_TEX_TILING_UIF_XOR) {
		const uint32_t ubh = 2u * utile_height(l->cpp);
		const uint32_t implicit = (l->height + ubh - 1u) / ubh * ubh;
		out.iis = s->padded_height / ubh;
		out.icfg |= (s->padded_height - implicit) / ubh << 22;
	}
	out.ioa = level0 | (3u + distance) << 3 | 1u;
	*j = out;
	return true;
}
