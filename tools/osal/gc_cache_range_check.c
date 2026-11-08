/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_cache_range_check.c - host check of src/osal/gc_cache_range.h, the test
 * that lets zephyr_gc.c skip data-cache maintenance on the non-cacheable
 * GPU carve-out.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Iinclude -Isrc/osal \
 *      tools/osal/gc_cache_range_check.c -o build/osal-check/gc_cache_range_check \
 *      && build/osal-check/gc_cache_range_check
 *
 * The shapes: ranges inside (whole view, middle, last byte); straddling
 * either end; wholly outside; empty ranges and an empty view; the top of the
 * address space, where base + size would wrap.
 */
#include <stdint.h>
#include <stdio.h>

#include "gc_cache_range.h"

#define PLANNED 20

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("gc_cache_range_check: FAIL %s\n", what);
	}
}

int main(void)
{
	const uintptr_t b = 0x40000000u;
	const size_t sz = 0x100000;
	const uintptr_t top = UINTPTR_MAX;

	/* inside */
	check(gc_cache_range_inside(b, sz, b, sz), "whole view refused");
	check(gc_cache_range_inside(b, sz, b + 4096, 4096), "middle refused");
	check(gc_cache_range_inside(b, sz, b + sz - 1, 1), "last byte refused");
	check(gc_cache_range_inside(b, sz, b, 1), "first byte refused");

	/* straddling and outside */
	check(!gc_cache_range_inside(b, sz, b - 1, 2), "straddling the start taken");
	check(!gc_cache_range_inside(b, sz, b + sz - 1, 2), "straddling the end taken");
	check(!gc_cache_range_inside(b, sz, b - 16, sz), "shifted down taken");
	check(!gc_cache_range_inside(b, sz, b, sz + 1), "one byte too long taken");
	check(!gc_cache_range_inside(b, sz, b - 4096, 4096), "just below taken");
	check(!gc_cache_range_inside(b, sz, b + sz, 1), "just above taken");
	check(!gc_cache_range_inside(b, sz, 0, 16), "NULL page taken");

	/* empty */
	check(gc_cache_range_inside(b, sz, b + 8, 0), "empty range inside refused");
	check(!gc_cache_range_inside(b, sz, b - 1, 0), "empty range below taken");
	check(!gc_cache_range_inside(b, sz, b + sz + 1, 0), "empty range above taken");
	check(!gc_cache_range_inside(0, 0, 0, 0), "empty view taken");
	check(!gc_cache_range_inside(b, 0, b, 1), "byte in an empty view taken");

	/* the top of the address space: no sum may wrap */
	check(gc_cache_range_inside(top - 4095, 4096, top - 15, 16), "range ending at the top refused");
	check(!gc_cache_range_inside(top - 4095, 4096, top - 15, 17), "range wrapping past the top taken");
	check(!gc_cache_range_inside(b, sz, b + 16, SIZE_MAX), "SIZE_MAX length taken");
	check(!gc_cache_range_inside(b, sz, top, 2), "start at the top, wrapping, taken");

	if (checks != PLANNED) {
		printf("gc_cache_range_check: %d checks run but %d planned\n", checks, PLANNED);
		return 1;
	}
	printf("gc_cache_range_check: %d checks run (planned %d), %d failed\n", checks, PLANNED,
	       failures);
	return failures ? 1 : 0;
}
