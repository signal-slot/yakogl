/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_timing.c - the time answers every Zephyr OSAL gives (zephyr.c,
 * zephyr_gc.c, zephyr_v3d.c), in one place so the three cannot drift apart:
 *
 *   OSAL_QUERY_CLOCK_NS    the 64-bit cycle counter in nanoseconds;
 *   OSAL_QUERY_DRAW_STATS  one timed scope's draw steps (src/draw_steps.rs),
 *                          added to the totals osal_draw_stats returns.
 *
 * Each OSAL's query op hands these two ids to osal_zephyr_timing_query.
 * Single-threaded by the GL contract, like the library that reports.
 */

#include <errno.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <zephyr/kernel.h>

#include "osal_internal.h"

/* src/draw_steps.rs OsalDrawStats has the same layout (its size is pinned
 * there to the same number) */
BUILD_ASSERT(sizeof(struct osal_draw_stats) == 8u * OSAL_DRAW_STEPS + 8u, "struct osal_draw_stats layout");

static struct {
	bool reported;
	struct osal_draw_stats total;
} s_draw;

static int clock_ns(void *out, size_t out_sz)
{
#ifdef CONFIG_TIMER_HAS_64BIT_CYCLE_COUNTER
	if (out == NULL || out_sz != sizeof(uint64_t)) {
		return -EINVAL;
	}
	/* whole seconds and the remainder apart: the counter times 1e9
	 * overflows 64 bits after about 341 s at the Pi 4's 54 MHz */
	const uint64_t hz = (uint64_t)sys_clock_hw_cycles_per_sec();
	const uint64_t cyc = k_cycle_get_64();

	if (hz == 0u) {
		return -ENOTSUP;
	}
	*(uint64_t *)out = cyc / hz * 1000000000ull + cyc % hz * 1000000000ull / hz;
	return 0;
#else
	(void)out;
	(void)out_sz;
	return -ENOTSUP;
#endif
}

static int draw_stats_add(const void *in, size_t in_sz)
{
	if (in == NULL || in_sz != sizeof(struct osal_draw_stats)) {
		return -EINVAL;
	}
	const struct osal_draw_stats *d = in;

	for (unsigned int i = 0; i < OSAL_DRAW_STEPS; i++) {
		s_draw.total.ns[i] += d->ns[i];
	}
	s_draw.total.untimed += d->untimed;
	s_draw.reported = true;
	return 0;
}

int osal_zephyr_timing_query(uint32_t what, void *out, size_t out_sz)
{
	switch (what) {
	case OSAL_QUERY_CLOCK_NS:
		return clock_ns(out, out_sz);
	case OSAL_QUERY_DRAW_STATS:
		return draw_stats_add(out, out_sz);
	default:
		return -ENOTSUP;
	}
}

bool osal_draw_stats(uint64_t ns[OSAL_DRAW_STEPS], uint32_t *untimed)
{
	if (!s_draw.reported) {
		return false;
	}
	if (ns != NULL) {
		memcpy(ns, s_draw.total.ns, sizeof(s_draw.total.ns));
	}
	if (untimed != NULL) {
		*untimed = s_draw.total.untimed;
	}
	return true;
}
