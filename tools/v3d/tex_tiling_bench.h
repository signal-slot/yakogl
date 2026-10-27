/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tex_tiling_bench.h: how long v3d_tex_store_level takes on the board, per
 * texture size, in the memory the backend tiles in
 * (docs/online-shaderc/v3d-backend.md, "Re-tiling cost per job").
 *
 * The backend re-tiles every sampled texture into a per-job shadow with
 * v3d_tex_store_level (src/v3d/texture.rs tile(), through the FFI). It reads
 * the GL core's linear RGBA8 storage and writes the shadow; both are in the
 * V3D OSAL's carve-out, CPU-mapped Normal non-cacheable
 * (src/osal/zephyr_v3d.c). The backend has no clock, so the cost per job is
 * the bytes it counts (v3d_texture_counters: the tiled image's size, padding
 * included) divided by the rate measured here.
 *
 * So the sample must pass buffers from the SAME memory (the OSAL's gpu_alloc,
 * or a Normal-NC mapping made the same way). Cached buffers would measure a
 * different thing and give a rate far too good.
 *
 * Header only, no OS calls: the caller passes its clock (k_cycle_get_64 on
 * Zephyr), the clock's rate, and its print function.
 */
#ifndef TEX_TILING_BENCH_H
#define TEX_TILING_BENCH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "v3d_tex.h"
#include "tex_pixel_offset.h" /* the checker's own texel addresses */

typedef uint64_t (*ttb_clock_fn)(void);
typedef void (*ttb_print_fn)(const char *fmt, ...);

struct ttb_result {
	uint32_t width, height;
	uint32_t tiled_bytes; /* the layout's size: what the backend's counters count */
	uint32_t runs;
	uint64_t best_cycles, worst_cycles;
	uint32_t checked_texels, wrong_texels;
};

/* A value for texel (x, y) that no other texel of the image has, so a texel
 * written to the wrong place is found. */
static inline uint32_t ttb_texel(uint32_t x, uint32_t y)
{
	return (x & 0xfffu) | (y & 0xfffu) << 12 | 0xa5000000u;
}

/* Check every texel of a tiled result against v3d_tex_pixel_offset: fills
 * r->checked_texels and r->wrong_texels (r->width and r->height say the size). */
static inline void ttb_verify(const uint8_t *dst, struct ttb_result *r)
{
	struct v3d_tex_layout l;
	r->checked_texels = r->wrong_texels = 0;
	if (!v3d_tex_layout(r->width, r->height, 1, 4, &l))
		return;
	const struct v3d_tex_slice *s = &l.slices[0];
	for (uint32_t y = 0; y < r->height; y++)
		for (uint32_t x = 0; x < r->width; x++) {
			const uint8_t *p = dst + s->offset +
					   v3d_tex_pixel_offset(s->tiling, 4, s->padded_height, x, y);
			uint32_t got = (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 |
				       (uint32_t)p[3] << 24;
			r->checked_texels++;
			if (got != ttb_texel(x, y))
				r->wrong_texels++;
		}
}

/* Tile a width x height RGBA8 level from `src` (rows width * 4 bytes apart)
 * into `dst` `runs` times, timing each run; then check every texel of the
 * result against v3d_tex_pixel_offset, so a run that wrote nothing, or wrote
 * the wrong place, cannot pass. False, with nothing measured, when a buffer is
 * too small or the size is refused. */
static inline bool ttb_measure(uint8_t *src, uint32_t src_size, uint8_t *dst, uint32_t dst_size,
			       uint32_t width, uint32_t height, uint32_t runs, ttb_clock_fn clock,
			       struct ttb_result *r)
{
	struct v3d_tex_layout l;
	if (!src || !dst || !clock || !r || runs == 0 || width == 0 ||
	    !v3d_tex_layout(width, height, 1, 4, &l) || src_size / 4u / width < height ||
	    dst_size < l.size)
		return false;

	for (uint32_t y = 0; y < height; y++)
		for (uint32_t x = 0; x < width; x++) {
			uint32_t v = ttb_texel(x, y);
			uint8_t *p = src + ((size_t)y * width + x) * 4u;
			p[0] = (uint8_t)v;
			p[1] = (uint8_t)(v >> 8);
			p[2] = (uint8_t)(v >> 16);
			p[3] = (uint8_t)(v >> 24);
		}

	r->width = width;
	r->height = height;
	r->tiled_bytes = l.size;
	r->runs = runs;
	r->best_cycles = UINT64_MAX;
	r->worst_cycles = 0;
	for (uint32_t i = 0; i < runs; i++) {
		uint64_t t0 = clock();
		v3d_tex_store_level(dst, &l, 0, src, width * 4u);
		uint64_t t = clock() - t0;
		if (t < r->best_cycles)
			r->best_cycles = t;
		if (t > r->worst_cycles)
			r->worst_cycles = t;
	}

	ttb_verify(dst, r);
	return true;
}

/* One line per size: the bytes, the best and worst run in microseconds, and
 * the best rate in KiB per millisecond (about MB/s); integers only, for a
 * printk without %f. True when every texel was checked and none was wrong. */
static inline bool ttb_report(const struct ttb_result *r, uint64_t clock_hz, ttb_print_fn print)
{
	uint64_t best_us = clock_hz ? r->best_cycles * 1000000u / clock_hz : 0;
	uint64_t worst_us = clock_hz ? r->worst_cycles * 1000000u / clock_hz : 0;
	uint64_t kib_per_ms = (r->best_cycles && clock_hz) ?
				      (uint64_t)r->tiled_bytes * clock_hz / r->best_cycles / 1024u /
					      1000u :
				      0;
	bool ok = r->checked_texels == r->width * r->height && r->wrong_texels == 0;
	print("tex_tiling %ux%u RGBA8: %u tiled bytes, %u runs, best %llu us, worst %llu us, "
	      "%llu KiB/ms; %u of %u texels checked, %u wrong: %s\n",
	      r->width, r->height, r->tiled_bytes, r->runs, (unsigned long long)best_us,
	      (unsigned long long)worst_us, (unsigned long long)kib_per_ms, r->checked_texels,
	      r->width * r->height, r->wrong_texels, ok ? "ok" : "FAIL");
	return ok;
}

/* The sizes: the kinds of texture the Slint cluster samples every frame
 * (docs/online-shaderc/v3d-backend.md "M7": one texture per Text item, sized
 * to the item; the logo; 256x1 gradients), up to a large layer. The largest
 * needs 2 MiB of source and 2 MiB of destination. */
#define TTB_SIZES 7
static const uint32_t ttb_sizes[TTB_SIZES][2] = {
	{ 256, 1 }, { 64, 16 }, { 128, 32 }, { 256, 64 }, { 256, 256 }, { 512, 128 }, { 1024, 512 },
};

#endif
