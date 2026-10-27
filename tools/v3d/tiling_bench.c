/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * tiling_bench: Mesa's per-pixel V3D tiling walk, its utile path and the
 * driver's UIF walker, on the texture shapes glmark2's refract, shadow and
 * terrain tile every frame (docs/notes/v3d-tiling-path.md). Build and run
 * with tools/v3d/tiling_bench.sh.
 *
 * It #includes Mesa's v3d_tiling.c (as driver/src/v3d/v3d_tiling_mesa.c does),
 * so it can call the file's static walkers directly:
 *
 *   per-pixel  v3d_move_pixels_unaligned over the whole box: one
 *              get_pixel_offset and one cpp-byte memcpy per texel. This is what
 *              the file does for the edge pixels of a box and for a box with no
 *              whole utile; here it is forced over everything.
 *   utile      Mesa's v3d_store_tiled_image: v3d_move_pixels_general_percpp,
 *              a v3d_store_utile per whole utile and the per-pixel walk only
 *              for the partial edges. v3d_tex_store_box called it for every
 *              box before the walker.
 *   walker     v3d_tex_store_level, the backend's call: the UIF walker of
 *              driver/src/v3d/v3d_tiling_uif.c for a UIF level with cpp 4 and
 *              a box of whole UIF blocks, Mesa's v3d_store_tiled_image for any
 *              other level.
 *
 * All three write a chain / depth texture laid out by v3d_tex_layout (the
 * layout the backend uses) and the results are compared byte for byte with
 * Mesa's. Each level says whether v3d_tex_store_box takes the walker. Then
 * the boxes that must fall back to Mesa (odd sizes, non-UIF levels, cpp other
 * than 4, boxes not of whole UIF blocks) are checked to be refused by the
 * walker and stored as Mesa stores them, and sub-boxes of whole UIF blocks
 * (inside a column, across columns, the odd columns of a UIF_XOR level) to be
 * taken by the walker with Mesa's bytes.
 *
 * The host's memory is cached and its CPU is not the board's Cortex-A72 with
 * a non-cacheable destination: the numbers say how the walks compare on the
 * host, not what the board spends.
 *
 * Usage: tiling_bench [runs]   (default 5; the best run of each is printed)
 *
 * Last line: "tiling_bench: N checks, 0 failed" when every compare and every
 * path held; exit 1 otherwise.
 */
#include "v3d_mesa_posix.h" /* first: see v3d_tiling_mesa.c */

#ifndef NDEBUG
#define NDEBUG
#endif
#pragma GCC diagnostic ignored "-Wpointer-arith"
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"
#pragma GCC diagnostic ignored "-Wpedantic"
#include "common/v3d_tiling.c"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "v3d_tex.h"

static double now_ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (double)ts.tv_sec * 1e3 + (double)ts.tv_nsec / 1e6;
}

static uint32_t minify(uint32_t v, uint32_t level)
{
	uint32_t m = v >> level;
	return m ? m : 1;
}

static int checks, failed;

static void check(int ok, const char *what)
{
	checks++;
	if (!ok) {
		failed++;
		printf("  FAIL: %s\n", what);
	}
}

/* The per-pixel walk over one level, for the slice's tiling. */
static void store_level_perpixel(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
				 const uint8_t *src, uint32_t src_stride)
{
	const struct v3d_tex_slice *s = &l->slices[level];
	struct pipe_box box = { .x = 0,
				.y = 0,
				.width = (int)minify(l->width, level),
				.height = (int)minify(l->height, level),
				.depth = 1 };
	uint32_t (*off)(uint32_t, uint32_t, uint32_t, uint32_t);
	switch (s->tiling) {
	case V3D_TILING_UIF_XOR:
		off = v3d_get_uif_xor_pixel_offset;
		break;
	case V3D_TILING_UIF_NO_XOR:
		off = v3d_get_uif_no_xor_pixel_offset;
		break;
	case V3D_TILING_UBLINEAR_2_COLUMN:
		off = v3d_get_ublinear_2_column_pixel_offset;
		break;
	case V3D_TILING_UBLINEAR_1_COLUMN:
		off = v3d_get_ublinear_1_column_pixel_offset;
		break;
	default:
		off = v3d_get_lt_pixel_offset;
		break;
	}
	v3d_move_pixels_unaligned(image + s->offset, s->stride, (void *)(uintptr_t)src, src_stride,
				  (int)l->cpp, s->padded_height, &box, off, false);
}

/* Mesa's v3d_store_tiled_image on a box of one level */
static void store_box_mesa(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			   uint32_t x, uint32_t y, uint32_t w, uint32_t h, const uint8_t *src,
			   uint32_t src_stride)
{
	const struct v3d_tex_slice *s = &l->slices[level];
	struct pipe_box box = {
		.x = (int)x, .y = (int)y, .width = (int)w, .height = (int)h, .depth = 1
	};
	v3d_store_tiled_image(image + s->offset, s->stride, (void *)(uintptr_t)src, src_stride,
			      (enum v3d_tiling_mode)s->tiling, (int)l->cpp, s->padded_height, &box);
}

static void store_level_utile(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			      const uint8_t *src, uint32_t src_stride)
{
	store_box_mesa(image, l, level, 0, 0, minify(l->width, level), minify(l->height, level),
		       src, src_stride);
}

static void store_level_walker(uint8_t *image, const struct v3d_tex_layout *l, uint32_t level,
			       const uint8_t *src, uint32_t src_stride)
{
	v3d_tex_store_level(image, l, level, src, src_stride);
}

/* Does v3d_tex_store_box take the walker for this box? The walker's own
 * answer, on a scratch image (it writes only when it takes the box). */
static int walker_takes(const struct v3d_tex_layout *l, uint32_t level, uint32_t x, uint32_t y,
			uint32_t w, uint32_t h, const uint8_t *src, uint32_t src_stride)
{
	uint8_t *scratch = calloc(1, l->size);
	if (!scratch)
		return 0;
	int taken = v3d_tex_store_uif4(scratch + l->slices[level].offset, &l->slices[level], l->cpp,
				       x, y, w, h, src, src_stride);
	free(scratch);
	return taken;
}

typedef void (*store_fn)(uint8_t *, const struct v3d_tex_layout *, uint32_t, const uint8_t *,
			 uint32_t);

/* Level L's source rows: a packed chain, level 0 first, each level its own
 * width * cpp pitch. */
struct chain {
	struct v3d_tex_layout l;
	uint8_t *src; /* packed levels */
	size_t src_off[V3D_TEX_MAX_LEVELS];
	size_t src_bytes;
};

static int chain_make(struct chain *c, uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp)
{
	if (!v3d_tex_layout(w, h, levels, cpp, &c->l))
		return 0;
	size_t off = 0;
	for (uint32_t i = 0; i < levels; i++) {
		c->src_off[i] = off;
		off += (size_t)minify(w, i) * minify(h, i) * cpp;
	}
	c->src_bytes = off;
	c->src = malloc(off);
	if (!c->src)
		return 0;
	uint32_t x = 0x9e3779b9u; /* a texel value no neighbour shares */
	for (size_t i = 0; i < off; i += 4) {
		x = x * 1664525u + 1013904223u;
		memcpy(c->src + i, &x, off - i < 4 ? off - i : 4);
	}
	return 1;
}

static void chain_store(const struct chain *c, uint8_t *dst, store_fn fn, uint32_t first,
			uint32_t last)
{
	for (uint32_t i = first; i <= last; i++)
		fn(dst, &c->l, i, c->src + c->src_off[i], minify(c->l.width, i) * c->l.cpp);
}

static double best_of(const struct chain *c, uint8_t *dst, store_fn fn, uint32_t first,
		      uint32_t last, int runs)
{
	double best = 1e30;
	for (int r = 0; r < runs; r++) {
		double t0 = now_ms();
		chain_store(c, dst, fn, first, last);
		double t = now_ms() - t0;
		if (t < best)
			best = t;
	}
	return best;
}

static const char *tiling_name(int t)
{
	switch (t) {
	case V3D_TILING_UIF_XOR:
		return "UIF_XOR";
	case V3D_TILING_UIF_NO_XOR:
		return "UIF";
	case V3D_TILING_UBLINEAR_2_COLUMN:
		return "UBL2";
	case V3D_TILING_UBLINEAR_1_COLUMN:
		return "UBL1";
	default:
		return "LT";
	}
}

static int is_uif(int t)
{
	return t == V3D_TILING_UIF_XOR || t == V3D_TILING_UIF_NO_XOR;
}

/* Time the walks on one texture, compare their bytes with Mesa's, and check
 * that the walker takes a level exactly when it is UIF, cpp is 4 and the
 * level's size is whole UIF blocks. */
static void bench(const char *name, uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp,
		  int runs, int perpixel)
{
	struct chain c;
	char what[200];
	if (!chain_make(&c, w, h, levels, cpp)) {
		printf("tiling_bench: %s: layout refused\n", name);
		check(0, name);
		return;
	}
	size_t n = c.l.size;
	uint8_t *ref = calloc(1, n), *b = calloc(1, n);
	if (!ref || !b) {
		check(0, "allocation");
		return;
	}

	printf("%s: %ux%u cpp %u, %u levels, %zu tiled bytes\n", name, w, h, cpp, levels, n);
	size_t walker_bytes = 0, total_bytes = 0;
	for (uint32_t i = 0; i < levels; i++) {
		uint32_t lw = minify(w, i), lh = minify(h, i);
		const struct v3d_tex_slice *s = &c.l.slices[i];
		int want = is_uif(s->tiling) && cpp == 4 && lw % 8 == 0 && lh % 8 == 0;
		int taken = walker_takes(&c.l, i, 0, 0, lw, lh, c.src + c.src_off[i], lw * cpp);
		total_bytes += (size_t)lw * lh * cpp;
		if (taken)
			walker_bytes += (size_t)lw * lh * cpp;
		printf("  level %2u %4ux%-4u %-7s %s\n", i, lw, lh, tiling_name(s->tiling),
		       taken ? "walker" : "Mesa (fallback)");
		snprintf(what, sizeof(what), "%s level %u: walker %s, expected %s", name, i,
			 taken ? "taken" : "refused", want ? "taken" : "refused");
		check(taken == want, what);
	}
	printf("  walker takes %.1f %% of the texels' bytes\n",
	       total_bytes ? 100.0 * (double)walker_bytes / (double)total_bytes : 0.0);

	double ut = best_of(&c, ref, store_level_utile, 0, levels - 1, runs);
	double wk = best_of(&c, b, store_level_walker, 0, levels - 1, runs);
	int same = memcmp(ref, b, n) == 0;
	snprintf(what, sizeof(what), "%s: walker bytes equal Mesa's", name);
	check(same, what);
	if (perpixel) {
		memset(b, 0, n);
		double pp = best_of(&c, b, store_level_perpixel, 0, levels - 1, runs);
		snprintf(what, sizeof(what), "%s: per-pixel bytes equal Mesa's", name);
		check(memcmp(ref, b, n) == 0, what);
		printf("  per-pixel walk  %8.3f ms\n", pp);
	}
	printf("  utile loop      %8.3f ms\n  walker          %8.3f ms  (%.2fx faster)\n"
	       "  bytes %s\n",
	       ut, wk, wk > 0 ? ut / wk : 0.0, same ? "identical" : "DIFFER");

	/* level 0 alone: the part of the chain that carries the bytes */
	memset(ref, 0, n);
	memset(b, 0, n);
	double ut0 = best_of(&c, ref, store_level_utile, 0, 0, runs);
	double wk0 = best_of(&c, b, store_level_walker, 0, 0, runs);
	same = memcmp(ref, b, n) == 0;
	snprintf(what, sizeof(what), "%s: level 0 walker bytes equal Mesa's", name);
	check(same, what);
	printf("  level 0 only: utile %.3f ms, walker %.3f ms (%.2fx), bytes %s\n", ut0, wk0,
	       wk0 > 0 ? ut0 / wk0 : 0.0, same ? "identical" : "DIFFER");

	free(ref);
	free(b);
	free(c.src);
}

/* One box of one level through v3d_tex_store_box and through Mesa: the same
 * bytes, and the walker taken or refused as `want_walker` says. */
static void box_case(uint32_t w, uint32_t h, uint32_t levels, uint32_t cpp, uint32_t level,
		     uint32_t bx, uint32_t by, uint32_t bw, uint32_t bh, int want_walker,
		     const char *why)
{
	struct chain c;
	char what[200];
	if (!chain_make(&c, w, h, levels, cpp)) {
		snprintf(what, sizeof(what), "%ux%u cpp %u: layout refused", w, h, cpp);
		check(0, what);
		return;
	}
	uint32_t stride = minify(w, level) * cpp;
	const uint8_t *src = c.src + c.src_off[level] + (size_t)by * stride + (size_t)bx * cpp;
	uint8_t *ref = calloc(1, c.l.size), *b = calloc(1, c.l.size);
	if (!ref || !b) {
		check(0, "allocation");
		return;
	}
	store_box_mesa(ref, &c.l, level, bx, by, bw, bh, src, stride);
	int ok = v3d_tex_store_box(b, &c.l, level, bx, by, bw, bh, src, stride);
	int taken = walker_takes(&c.l, level, bx, by, bw, bh, src, stride);
	int same = ok && memcmp(ref, b, c.l.size) == 0;
	printf("  %4ux%-4u cpp %2u level %u %-7s box %u,%u %ux%u: %-6s bytes %s  (%s)\n", w, h,
	       cpp, level, tiling_name(c.l.slices[level].tiling), bx, by, bw, bh,
	       taken ? "walker" : "Mesa", same ? "identical" : "DIFFER", why);
	snprintf(what, sizeof(what), "%ux%u cpp %u level %u box %u,%u %ux%u: %s", w, h, cpp, level,
		 bx, by, bw, bh, why);
	check(same && taken == want_walker, what);
	free(ref);
	free(b);
	free(c.src);
}

int main(int argc, char **argv)
{
	int runs = argc > 1 ? atoi(argv[1]) : 5;
	if (runs < 1)
		runs = 1;
#if DETECT_ARCH_AARCH64
	printf("tiling_bench: DETECT_ARCH_AARCH64: v3d_store_utile is the st1 code\n");
#elif DETECT_ARCH_ARM
	printf("tiling_bench: DETECT_ARCH_ARM: NEON only with V3D_BUILD_NEON\n");
#else
	printf("tiling_bench: not an ARM build: v3d_store_utile is the memcpy loop\n");
#endif
	/* the glmark2 shapes (refract's chain and depth texture, terrain's
	 * chain) and a power-of-two chain whose big levels are UIF_XOR */
	bench("1600x1200 RGBA8, 11-level chain", 1600, 1200, 11, 4, runs, 1);
	bench("1600x1200 32-bit depth, level 0", 1600, 1200, 1, 4, runs, 1);
	bench("800x600 RGBA8, 10-level chain", 800, 600, 10, 4, runs, 1);
	bench("256x256 RGBA8, 9-level chain", 256, 256, 9, 4, runs, 1);
	/* odd sizes: UIF levels that are not whole UIF blocks fall back */
	bench("1000x750 RGBA8, 10-level chain", 1000, 750, 10, 4, runs, 0);
	bench("257x255 RGBA8, 9-level chain", 257, 255, 9, 4, runs, 0);
	bench("1366x766 RGBA8, 1 level", 1366, 766, 1, 4, runs, 0);

	printf("boxes:\n");
	/* the walker: boxes of whole UIF blocks of a UIF level, cpp 4 */
	box_case(1600, 1200, 1, 4, 0, 8, 16, 64, 40, 1, "inside, from a column's second block");
	box_case(1600, 1200, 1, 4, 0, 24, 8, 80, 1184, 1, "across columns, from a column's last block");
	box_case(256, 256, 1, 4, 0, 0, 0, 256, 256, 1, "UIF_XOR, the whole level");
	box_case(256, 256, 1, 4, 0, 32, 120, 64, 16, 1, "UIF_XOR, odd column, rows 15 and 16");
	box_case(512, 512, 2, 4, 1, 40, 128, 168, 120, 1, "UIF_XOR level 1");
	box_case(256, 256, 9, 4, 3, 0, 0, 32, 32, 1, "UIF level 3, 32x32");
	/* Mesa: boxes not of whole UIF blocks */
	box_case(1600, 1200, 1, 4, 0, 4, 0, 64, 64, 0, "x not a multiple of 8");
	box_case(1600, 1200, 1, 4, 0, 0, 4, 64, 64, 0, "y not a multiple of 8");
	box_case(1600, 1200, 1, 4, 0, 0, 0, 60, 64, 0, "width not a multiple of 8");
	box_case(1600, 1200, 1, 4, 0, 0, 0, 64, 1196, 0, "height not a multiple of 8");
	box_case(256, 256, 1, 4, 0, 3, 5, 251, 250, 0, "UIF_XOR, unaligned");
	/* Mesa: not UIF */
	box_case(256, 256, 9, 4, 4, 0, 0, 16, 16, 0, "UBLINEAR 2 column level");
	box_case(256, 256, 9, 4, 5, 0, 0, 8, 8, 0, "UBLINEAR 1 column level");
	box_case(256, 256, 9, 4, 6, 0, 0, 4, 4, 0, "LT level");
	/* Mesa: cpp other than 4 */
	box_case(256, 256, 1, 1, 0, 0, 0, 256, 256, 0, "cpp 1");
	box_case(256, 256, 1, 2, 0, 0, 0, 256, 256, 0, "cpp 2");
	box_case(256, 256, 1, 8, 0, 0, 0, 256, 256, 0, "cpp 8");
	box_case(256, 256, 1, 16, 0, 0, 0, 256, 256, 0, "cpp 16");

	printf("tiling_bench: %d checks, %d failed\n", checks, failed);
	return failed ? 1 : 0;
}
