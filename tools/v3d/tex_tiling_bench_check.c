/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * tex_tiling_bench_check: checks tools/v3d/tex_tiling_bench.h, the board
 * harness that times v3d_tex_store_level, on the host. The host's rates mean
 * nothing (cached memory, another CPU); what is checked is the harness:
 *
 *  1. Every size of ttb_sizes runs, and every texel of every result is
 *     checked and right.
 *  2. The refusals: a destination one byte short of the layout, a source one
 *     row short, zero runs.
 *  3. ttb_verify finds one wrong texel, and a destination that was never
 *     written.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include -Itools/v3d \
 *      tools/v3d/tex_tiling_bench_check.c driver/src/v3d/v3d_tex.c driver/src/v3d/v3d_tiling_mesa.c \
 *      -lm -o build/tex_tiling_bench_check
 *
 * Exit status 0 only when every check passed AND the number run is the number
 * planned.
 */
#define _POSIX_C_SOURCE 199309L
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "tex_tiling_bench.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("tex_tiling_bench_check: FAIL %s\n", what);
	}
}

static void print_out(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
}

static uint64_t clock_ns(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (uint64_t)ts.tv_sec * 1000000000u + (uint64_t)ts.tv_nsec;
}

static uint32_t tiled_size(uint32_t w, uint32_t h)
{
	struct v3d_tex_layout l;
	return v3d_tex_layout(w, h, 1, 4, &l) ? l.size : 0;
}

int main(void)
{
	/* 7 sizes x 2 (measured, reported ok); 3 refusals; 2 verify controls */
	const int planned = TTB_SIZES * 2 + 3 + 2;
	char what[160];

	for (int i = 0; i < TTB_SIZES; i++) {
		uint32_t w = ttb_sizes[i][0], h = ttb_sizes[i][1];
		uint32_t src_size = w * h * 4u, dst_size = tiled_size(w, h);
		uint8_t *src = malloc(src_size), *dst = calloc(1, dst_size);
		struct ttb_result r;
		snprintf(what, sizeof(what), "%ux%u: measured", w, h);
		bool measured = ttb_measure(src, src_size, dst, dst_size, w, h, 3, clock_ns, &r);
		check(measured, what);
		snprintf(what, sizeof(what), "%ux%u: every texel checked and right", w, h);
		check(measured && ttb_report(&r, 1000000000u, print_out), what);
		free(src);
		free(dst);
	}

	/* the refusals, on 64x16 */
	uint32_t w = 64, h = 16, src_size = w * h * 4u, dst_size = tiled_size(w, h);
	uint8_t *src = malloc(src_size), *dst = calloc(1, dst_size);
	struct ttb_result r;
	check(!ttb_measure(src, src_size, dst, dst_size - 1, w, h, 3, clock_ns, &r),
	      "a destination one byte short of the layout is refused");
	check(!ttb_measure(src, src_size - w * 4u, dst, dst_size, w, h, 3, clock_ns, &r),
	      "a source one row short is refused");
	check(!ttb_measure(src, src_size, dst, dst_size, w, h, 0, clock_ns, &r),
	      "zero runs are refused");

	/* ttb_verify can fail */
	ttb_measure(src, src_size, dst, dst_size, w, h, 1, clock_ns, &r);
	struct v3d_tex_layout l;
	v3d_tex_layout(w, h, 1, 4, &l);
	dst[l.slices[0].offset +
	    v3d_tex_pixel_offset(l.slices[0].tiling, 4, l.slices[0].padded_height, 5, 7)] ^= 1;
	ttb_verify(dst, &r);
	snprintf(what, sizeof(what), "one wrong texel is found (%u wrong)", r.wrong_texels);
	check(r.wrong_texels == 1 && r.checked_texels == w * h, what);
	memset(dst, 0, dst_size);
	ttb_verify(dst, &r);
	snprintf(what, sizeof(what), "a destination never written is found (%u wrong)",
		 r.wrong_texels);
	check(r.wrong_texels == w * h, what);
	free(src);
	free(dst);

	printf("tex_tiling_bench_check: %d checks run (planned %d), %d failed\n", checks, planned,
	       failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
