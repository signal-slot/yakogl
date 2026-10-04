/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * zephyr_clock.h - the answer every Zephyr OSAL gives OSAL_QUERY_CLOCK_NS
 * (osal_internal.h): the 64-bit cycle counter in nanoseconds. One copy, so
 * the three OSALs (zephyr.c, zephyr_gc.c, zephyr_v3d.c) cannot drift apart.
 */
#ifndef ZEPHYR_CLOCK_H
#define ZEPHYR_CLOCK_H

#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <zephyr/kernel.h>

/* 0 with the clock in *out, -ENOTSUP without a 64-bit counter or a rate. */
static inline int osal_zephyr_clock_ns(void *out, size_t out_sz)
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

#endif /* ZEPHYR_CLOCK_H */
