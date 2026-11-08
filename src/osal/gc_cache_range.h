/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_cache_range.h - is a CPU range wholly inside the GPU carve-out's CPU
 * view? The carve-out is mapped Normal non-cacheable (driver/src/vivante/
 * gc_render.c), so data-cache maintenance on a range inside it does nothing
 * but cost CPU time; zephyr_gc.c skips it there and keeps it for anything
 * else. Plain C with no OS in it: tools/osal/gc_cache_range_check.c checks it.
 *
 * Nothing wraps: `base + size` and `cpu + n` are never computed.
 */
#ifndef GC_CACHE_RANGE_H
#define GC_CACHE_RANGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* True when [cpu, cpu + n) lies inside [base, base + size). An empty view
 * holds nothing; an empty range (n == 0) inside a non-empty view is inside,
 * at cpu == base + size included. */
static inline bool gc_cache_range_inside(uintptr_t base, size_t size, uintptr_t cpu, size_t n)
{
	if (size == 0)
		return false;
	if (cpu < base)
		return false;
	return cpu - base <= size && n <= size - (cpu - base);
}

#endif /* GC_CACHE_RANGE_H */
