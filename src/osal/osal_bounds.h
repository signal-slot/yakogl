/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_bounds.h - the one argument check of the OSAL gpu_write / gpu_read ops
 * (include/gles_osal.h states the contract). Every OSAL (zephyr.c,
 * zephyr_gc.c, zephyr_v3d.c, baremetal.c, linux.c) calls
 * it before its memcpy, so all six refuse the same arguments. Plain C with
 * no OS in it, so the host checks it (tools/osal_bounds_check.c).
 *
 * The range test is written so that nothing in it can wrap: `off + n` wraps
 * for a large `n` (off 16, n SIZE_MAX - 7 gives 8), passes a
 * `off + n > size` test and sends the memcpy past the allocation. With
 * `off > size` tested first, `size - off` cannot wrap.
 */
#ifndef OSAL_BOUNDS_H
#define OSAL_BOUNDS_H

#include <stdbool.h>
#include <stddef.h>

#include "gles_osal.h"

/* True when gpu_write / gpu_read may copy `n` bytes between `buf` and
 * [off, off + n) of `mem`: `mem` and its CPU view exist, `buf` is not NULL
 * (also for n == 0: memcpy takes no null pointer, C11 7.24.1p2) and the
 * range lies inside mem->size. n == 0 at off == mem->size is in range. */
static inline bool osal_gpu_copy_ok(const struct osal_gpu_mem *mem, size_t off,
				    const void *buf, size_t n)
{
	if (!mem || !mem->cpu || !buf)
		return false;
	if (off > mem->size || n > mem->size - off)
		return false;
	return true;
}

#endif /* OSAL_BOUNDS_H */
