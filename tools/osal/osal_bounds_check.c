/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_bounds_check.c - host check of src/osal/osal_bounds.h, the argument
 * check every OSAL's gpu_write / gpu_read makes before its memcpy (seven
 * files, most of which no host build reaches).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Iinclude -Isrc/osal \
 *      tools/osal/osal_bounds_check.c -o build/osal-check/osal_bounds_check \
 *      && build/osal-check/osal_bounds_check
 *
 * The shapes:
 * 1. In range: the whole allocation, a middle piece, the last byte.
 * 2. The edges: n == 0 at off 0 and at off == size (in range); one byte at
 *    off == size, n == size + 1, off == size + 1 and one byte past the end
 *    (out).
 * 3. Wrap: off + n wraps past SIZE_MAX to a small value that an
 *    `off + n > size` test lets through (off 16, n SIZE_MAX - 7 wraps to 8;
 *    off SIZE_MAX, n 1 wraps to 0). Every one is refused, and the old test
 *    is asserted to have let it through, so each case is a real wrap.
 * 4. NULL: mem, mem->cpu and the caller's buffer, also with n == 0.
 */
#include <stdint.h>
#include <stdio.h>

#include "osal_bounds.h"

#define PLANNED 21

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("osal_bounds_check: FAIL %s\n", what);
	}
}

/* The test the OSALs had before osal_bounds.h: it passes a wrapped off + n. */
static bool old_test_passes(size_t size, size_t off, size_t n)
{
	return !(off + n > size);
}

int main(void)
{
	static uint8_t backing[4096];
	uint8_t buf[1];
	struct osal_gpu_mem mem = { .cpu = backing, .gpu = 0x100000, .phys = 0,
				    .size = sizeof(backing), .impl = NULL };
	const size_t size = mem.size;

	/* 1. in range */
	check(osal_gpu_copy_ok(&mem, 0, buf, size), "whole allocation refused");
	check(osal_gpu_copy_ok(&mem, 100, buf, 200), "middle piece refused");
	check(osal_gpu_copy_ok(&mem, size - 1, buf, 1), "last byte refused");

	/* 2. edges */
	check(osal_gpu_copy_ok(&mem, 0, buf, 0), "n == 0 at off 0 refused");
	check(osal_gpu_copy_ok(&mem, size, buf, 0), "n == 0 at off == size refused");
	check(!osal_gpu_copy_ok(&mem, size, buf, 1), "one byte at off == size accepted");
	check(!osal_gpu_copy_ok(&mem, 0, buf, size + 1), "n == size + 1 accepted");
	check(!osal_gpu_copy_ok(&mem, size + 1, buf, 0), "off == size + 1 accepted");
	check(!osal_gpu_copy_ok(&mem, 1, buf, size), "one byte past the end accepted");

	/* 3. wrap: each case is one the old test let through */
	check(old_test_passes(size, 16, SIZE_MAX - 7), "off 16 + n SIZE_MAX-7 does not wrap");
	check(!osal_gpu_copy_ok(&mem, 16, buf, SIZE_MAX - 7), "off 16, n SIZE_MAX-7 accepted");
	check(old_test_passes(size, SIZE_MAX, 1), "off SIZE_MAX + n 1 does not wrap");
	check(!osal_gpu_copy_ok(&mem, SIZE_MAX, buf, 1), "off SIZE_MAX, n 1 accepted");
	check(old_test_passes(size, SIZE_MAX - size + 1, size),
	      "off SIZE_MAX-size+1 + n size does not wrap");
	check(!osal_gpu_copy_ok(&mem, SIZE_MAX - size + 1, buf, size),
	      "off SIZE_MAX-size+1, n size accepted");
	check(!osal_gpu_copy_ok(&mem, SIZE_MAX, buf, SIZE_MAX), "off SIZE_MAX, n SIZE_MAX accepted");

	/* 4. NULL */
	check(!osal_gpu_copy_ok(NULL, 0, buf, 1), "NULL mem accepted");
	check(!osal_gpu_copy_ok(&mem, 0, NULL, 1), "NULL buffer accepted");
	check(!osal_gpu_copy_ok(&mem, 0, NULL, 0), "NULL buffer with n == 0 accepted");
	mem.cpu = NULL;
	check(!osal_gpu_copy_ok(&mem, 0, buf, 1), "mem without a CPU view accepted");
	check(!osal_gpu_copy_ok(&mem, 0, buf, 0), "mem without a CPU view, n == 0 accepted");

	printf("osal_bounds_check: %d checks run (planned %d), %d failed\n", checks, PLANNED, failures);
	return failures || checks != PLANNED;
}
