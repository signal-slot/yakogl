/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of rpi5_scanout/hvs6_chan.h: write order and values of the HVS6
 * channel setup, the error cases and the output-to-FIFO map.
 *
 * Build: cc -std=c99 -Wall -Wextra -Werror -I../rpi5_scanout hvs6_chan_mock.c
 * (tools/v3d/check71.d/hvs6_chan.sh). Prints one line per check, then
 * "hvs6_chan_mock: N of N checks, 0 failed"; exits nonzero on a failure.
 */
#include <stdio.h>

#include "hvs6_chan.h"

static int total;
static int failed;

static void check(int ok, const char *what)
{
	total++;
	if (!ok)
		failed++;
	printf("%s: %s\n", ok ? "ok" : "FAIL", what);
}

static int w_is(const struct yd5_hvs6_log *l, unsigned int i, uint32_t off, uint32_t val)
{
	return i < l->n && l->w[i].offset == off && l->w[i].value == val;
}

static void check_enable(enum yd5_hvs6_variant v, const char *tag, uint32_t w, uint32_t h,
			 uint32_t ctrl0_on, uint32_t ctrl0_off, uint32_t ctrl1,
			 uint32_t bgnd, uint32_t lptrs)
{
	struct yd5_hvs6_log l;
	char name[96];
	int r;

	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_enable_v(&l, v, 0, w, h, 32, 0x00123456u);
	snprintf(name, sizeof(name), "%s %ux%u: returns 0, 7 writes", tag, w, h);
	check(r == 0 && l.n == 7, name);
	snprintf(name, sizeof(name), "%s %ux%u: stop first (ENB|RESET, then RESET)", tag, w, h);
	check(w_is(&l, 0, ctrl0_off, YD5_HVS6_CTRL0_ENB | YD5_HVS6_CTRL0_RESET) &&
	      w_is(&l, 1, ctrl0_off, YD5_HVS6_CTRL0_RESET), name);
	snprintf(name, sizeof(name), "%s %ux%u: LPTRS = 32 words", tag, w, h);
	check(w_is(&l, 2, lptrs, 32), name);
	snprintf(name, sizeof(name), "%s %ux%u: BGND = 0x123456", tag, w, h);
	check(w_is(&l, 3, bgnd, 0x00123456u), name);
	snprintf(name, sizeof(name), "%s %ux%u: CTRL0 = RESET", tag, w, h);
	check(w_is(&l, 4, ctrl0_off, YD5_HVS6_CTRL0_RESET), name);
	snprintf(name, sizeof(name), "%s %ux%u: CTRL1 = BGENB", tag, w, h);
	check(w_is(&l, 5, ctrl1, YD5_HVS6_CTRL1_BGENB), name);
	snprintf(name, sizeof(name), "%s %ux%u: CTRL0 = ENB|FWIDTH|LINES", tag, w, h);
	check(w_is(&l, 6, ctrl0_on, 0x80000000u | ((w - 1u) << 16) | (h - 1u)), name);
}

int main(void)
{
	struct yd5_hvs6_log l;
	enum yd5_hvs6_variant v = YD5_HVS6_C0;
	unsigned int o;
	int r;

	check_enable(YD5_HVS6_C0, "C0", 1920, 1080, 0x30, 0x30, 0x34, 0x38, 0x3c);
	check_enable(YD5_HVS6_C0, "C0", 640, 480, 0x30, 0x30, 0x34, 0x38, 0x3c);
	check_enable(YD5_HVS6_D0, "D0", 1920, 1080, 0x100, 0x100, 0x104, 0x108, 0x110);

	/* literal values, not derived from the macros */
	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_enable(&l, 0, 1920, 1080, 32, 0);
	check(r == 0 && l.w[6].value == 0x877f0437u,
	      "1920x1080 CTRL0 literal 0x877f0437");
	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_enable(&l, 0, 640, 480, 32, 0);
	check(r == 0 && l.w[6].value == 0x827f01dfu, "640x480 CTRL0 literal 0x827f01df");

	/* channels 1 and 2 use the stride */
	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_enable(&l, 2, 640, 480, 100, 0);
	check(r == 0 && l.w[0].offset == 0x70 && l.w[2].offset == 0x7c && l.w[2].value == 100,
	      "C0 channel 2: CTRL0 0x70, LPTRS 0x7c, list 100");
	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_enable_v(&l, YD5_HVS6_D0, 1, 640, 480, 100, 0);
	check(r == 0 && l.w[0].offset == 0x140 && l.w[2].offset == 0x150,
	      "D0 channel 1: CTRL0 0x140, LPTRS 0x150");

	/* errors append nothing */
	yd5_hvs6_log_init(&l);
	check(yd5_hvs6_chan_enable(&l, 3, 640, 480, 32, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: channel 3");
	check(yd5_hvs6_chan_enable(&l, 0, 0, 480, 32, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: zero width");
	check(yd5_hvs6_chan_enable(&l, 0, 640, 0, 32, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: zero height");
	check(yd5_hvs6_chan_enable(&l, 0, 640, 480, 31, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: list offset 31 (below the bootloader end)");
	check(yd5_hvs6_chan_enable(&l, 0, 640, 480, 0, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: list offset 0");
	check(yd5_hvs6_chan_enable(&l, 0, 640, 480, 32, 0) == 0 && l.n == 7,
	      "list offset 32 is accepted");
	yd5_hvs6_log_init(&l);
	check(yd5_hvs6_chan_enable(&l, 0, 8193, 480, 32, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: width 8193 (13-bit field)");
	check(yd5_hvs6_chan_enable(&l, 0, 640, 480, 4096, 0) == YD5_HVS6_EINVAL && l.n == 0,
	      "error: list offset 4096 (12-bit field)");
	check(yd5_hvs6_chan_enable(&l, 0, 8192, 8192, 4095, 0) == 0 &&
	      l.w[6].value == 0x9fff1fffu, "8192x8192, list 4095 accepted, CTRL0 0x9fff1fff");

	/* log overflow */
	yd5_hvs6_log_init(&l);
	l.n = YD5_HVS6_LOG_MAX - 6;
	r = yd5_hvs6_chan_enable(&l, 0, 640, 480, 32, 0);
	check(r == YD5_HVS6_ENOSPC && l.n == YD5_HVS6_LOG_MAX - 6, "error: log full appends nothing");

	/* disable */
	yd5_hvs6_log_init(&l);
	r = yd5_hvs6_chan_disable(&l, 1);
	check(r == 0 && l.n == 2 && w_is(&l, 0, 0x50, 0xc0000000u) && w_is(&l, 1, 0x50, 0x40000000u),
	      "disable channel 1 (C0): CTRL0 0x50 <- 0xc0000000, 0x40000000");
	yd5_hvs6_log_init(&l);
	check(yd5_hvs6_chan_disable(&l, 3) == YD5_HVS6_EINVAL && l.n == 0, "error: disable channel 3");

	/* version */
	check(yd5_hvs6_variant_from_version(0x12345653u, &v) == 0 && v == YD5_HVS6_C0, "version 0x53 is C0");
	check(yd5_hvs6_variant_from_version(0x54, &v) == 0 && v == YD5_HVS6_D0, "version 0x54 is D0");
	check(yd5_hvs6_variant_from_version(0x52, &v) == -1, "version 0x52 is rejected");

	/* FIFO map */
	check(yd5_hvs6_fifo_from_output(0) == 0, "FIFO: output 0 -> 0");
	check(yd5_hvs6_fifo_from_output(1) == 1, "FIFO: output 1 -> 1");
	check(yd5_hvs6_fifo_from_output(2) == 2, "FIFO: output 2 -> 2");
	check(yd5_hvs6_fifo_from_output(3) == 1, "FIFO: output 3 -> 1");
	check(yd5_hvs6_fifo_from_output(4) == 1, "FIFO: output 4 -> 1");
	for (o = 5; o < 8; o++)
		check(yd5_hvs6_fifo_from_output(o) == YD5_HVS6_EPIPE, "FIFO: output 5..7 -> -EPIPE");

	printf("hvs6_chan_mock: %d of %d checks, %d failed\n", total - failed, total, failed);
	return failed ? 1 : 0;
}
