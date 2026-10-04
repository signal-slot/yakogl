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

/* v3d_resource.c v3d_get_dimension_mpad, block size 1, level >= 1 */
static uint32_t dimension_mpad(uint32_t dimension, uint32_t level)
{
	uint32_t pot_dim = next_pot(minify(dimension, 1));
	return minify(pot_dim, level - 1);
}

/* v3d_resource.c v3d_setup_slices, for a 2D, one-layer, uncompressed,
 * never-shared resource (no winsys stride); `uif_top` as Mesa's: level 0 is
 * UIF whatever its size. */
static bool layout(uint32_t width, uint32_t height, uint32_t levels, uint32_t cpp, bool uif_top,
		   struct v3d_tex_layout *l);

/* ... single-sampled: uif_top false */
bool v3d_tex_layout(uint32_t width, uint32_t height, uint32_t levels, uint32_t cpp,
		    struct v3d_tex_layout *l)
{
	return layout(width, height, levels, cpp, false, l);
}

/* A multisampled (4x) width x height resource: one level of twice the width
 * and height, UIF at the top (v3d_resource.c:612-642: "MSAA
 * textures/renderbuffers are always laid out as single-level UIF",
 * `uif_top |= msaa`, `level_width *= 2; level_height *= 2`). */
bool v3d_tex_layout_msaa(uint32_t width, uint32_t height, uint32_t cpp, struct v3d_tex_layout *l)
{
	if (width == 0 || height == 0 || width >= (1u << 13) || height >= (1u << 13))
		return false;
	return layout(2 * width, 2 * height, 1, cpp, true, l);
}

static bool layout(uint32_t width, uint32_t height, uint32_t levels, uint32_t cpp, bool uif_top,
		   struct v3d_tex_layout *l)
{
	if (!l || width == 0 || height == 0 || width >= (1u << 14) || height >= (1u << 14) ||
	    levels == 0 || levels > V3D_TEX_MAX_LEVELS ||
	    (cpp != 1 && cpp != 2 && cpp != 4 && cpp != 8 && cpp != 16))
		return false;
	memset(l, 0, sizeof(*l));
	l->width = width;
	l->height = height;
	l->levels = levels;
	l->cpp = cpp;

	uint32_t offset = 0;
	uint32_t utile_w = utile_width(cpp);
	uint32_t utile_h = utile_height(cpp);
	uint32_t uif_block_w = utile_w * 2;
	uint32_t uif_block_h = utile_h * 2;

	/* "power-of-two padding is based on level 1" */
	uint32_t pot_width = 2 * dimension_mpad(width, 1);
	uint32_t pot_height = 2 * dimension_mpad(height, 1);

	for (int i = (int)levels - 1; i >= 0; i--) {
		struct v3d_tex_slice *slice = &l->slices[i];
		uint32_t level_width, level_height;

		if (i < 2) {
			level_width = minify(width, (uint32_t)i);
			level_height = minify(height, (uint32_t)i);
		} else {
			level_width = minify(pot_width, (uint32_t)i);
			level_height = minify(pot_height, (uint32_t)i);
		}

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

		slice->offset = offset;
		slice->stride = level_width * cpp;
		slice->padded_height = level_height;
		slice->size = level_height * slice->stride;

		uint32_t slice_total_size = slice->size;

		/* The HW aligns level 1's base to a page if any of level 1 or
		 * below could be UIF XOR.  The lower levels then inherit the
		 * alignment for as long as necessary, thanks to being power of
		 * two aligned. */
		if (i == 1 && level_width > 4 * uif_block_w &&
		    level_height > PAGE_CACHE_MINUS_1_5_UB_ROWS * uif_block_h)
			slice_total_size = align_u32(slice_total_size, V3D_UIFCFG_PAGE_SIZE);

		offset += slice_total_size;
	}
	l->size = offset;

	/* UIF/UBLINEAR levels need to be aligned to UIF-blocks, and LT only
	 * needs to be aligned to utile boundaries.  Since tiles are laid out
	 * from small to big in memory, we need to align the later UIF slices
	 * to UIF blocks, if they were preceded by non-UIF-block-aligned LT
	 * slices.
	 *
	 * We additionally align to 4k, which improves UIF XOR performance. */
	uint32_t page_align_offset = align_u32(l->slices[0].offset, 4096) - l->slices[0].offset;
	if (page_align_offset) {
		l->size += page_align_offset;
		for (uint32_t i = 0; i < levels; i++)
			l->slices[i].offset += page_align_offset;
	}
	return true;
}

/* ---- the texel tiling: Mesa's v3d_store_tiled_image ----------------------- */

/* The box is checked here, then handed to Mesa's v3d_store_tiled_image
 * (broadcom/common/v3d_tiling.c, compiled by v3d_tiling_mesa.c), with the
 * level's start, its padded height and the slice's tiling, whose enum is
 * Mesa's enum v3d_tiling_mode numbering (v3d_tex.h). */
bool v3d_tex_store_box(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		       uint32_t x, uint32_t y, uint32_t w, uint32_t h, const uint8_t *src,
		       uint32_t src_stride)
{
	if (!image || !l || !src || level >= l->levels)
		return false;
	uint32_t lw = minify(l->width, level), lh = minify(l->height, level);
	/* written so that no sum can wrap */
	if (x > lw || w > lw - x || y > lh || h > lh - y)
		return false;
	if (w == 0 || h == 0)
		return true;
	if (src_stride < w * l->cpp)
		return false;

	const struct v3d_tex_slice *s = &l->slices[level];
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
	return true;
}

/* v3d_tex_store_box's checks, then Mesa's v3d_load_tiled_image: the image
 * is only read (v3d_move_tiled_image with is_load true reads the tiled
 * side), the raster rows written. */
bool v3d_tex_load_box(const uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
		      uint32_t x, uint32_t y, uint32_t w, uint32_t h, uint8_t *dst,
		      uint32_t dst_stride)
{
	if (!image || !l || !dst || level >= l->levels)
		return false;
	uint32_t lw = minify(l->width, level), lh = minify(l->height, level);
	if (x > lw || w > lw - x || y > lh || h > lh - y)
		return false;
	if (w == 0 || h == 0)
		return true;
	if (dst_stride < w * l->cpp)
		return false;

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
	return true;
}

void v3d_tex_store_level(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			 const uint8_t *src, uint32_t src_stride)
{
	if (!l || level >= l->levels)
		return;
	(void)v3d_tex_store_box(image, l, level, 0, 0, minify(l->width, level),
				minify(l->height, level), src, src_stride);
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

/* ---- the source formats ---------------------------------------------------- */

/* Mesa's format table rows (v3dx_format_table.c, V3D 4.2):
 *   FORMAT(B8G8R8A8_UNORM, RGBA8, RGBA8, SWIZ_ZYXW, 16, 0)
 *   FORMAT(R8G8B8A8_UNORM, RGBA8, RGBA8, SWIZ_XYZW, 16, 0)
 *   FORMAT(A8_UNORM,       NO,    R8,    SWIZ_000X, 16, 0)
 *   FORMAT(L8_UNORM,       NO,    R8,    SWIZ_XXX1, 16, 0)
 *   FORMAT(L8A8_UNORM,     NO,    RG8,   SWIZ_XXXY, 16, 0)
 *   FORMAT(R32_FLOAT,      R32F,  R32F,  SWIZ_X001, 32, 1)   (v3dx_format_table.c:91)
 * with the swizzle translated as v3d_translate_pipe_swizzle does (v3d_util.c:
 * X..W -> 2..5). A row with cpp 0 is a format this cut refuses. */
static const struct {
	const char *name;
	struct v3d_tex_format f;
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
	[V3D_TEX_FORMAT_RGB565] = { "RGB565", { 0 } },
	[V3D_TEX_FORMAT_RGBA4] = { "RGBA4", { 0 } },
	[V3D_TEX_FORMAT_RGB5_A1] = { "RGB5_A1", { 0 } },
	/* return size 32: a look-up of it must set TMU Config Parameter 1's
	 * "Output Type 32-bit" (v3d_packet.xml) and read a word per channel */
	[V3D_TEX_FORMAT_R32F] = { "R32F",
				  { V3D_TEXTURE_DATA_FORMAT_R32F, 4,
				    { V3D_SWIZZLE_RED, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE },
				    32 } },
};

bool v3d_tex_format_get(enum v3d_tex_format_id id, struct v3d_tex_format *out)
{
	if ((unsigned)id >= V3D_TEX_FORMAT_COUNT || !out || formats[id].f.cpp == 0)
		return false;
	*out = formats[id].f;
	return true;
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
	t.srgb = s->srgb;
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
	if ((ioa & 0x3fu) != 0)
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
