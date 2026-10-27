/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The two calls of driver/src/vivante/gc_render.h that board_imx8mp.c makes,
 * for the host model (../latch_model.c defines them over the C heap). */
#ifndef LATCH_MODEL_GC_RENDER_H
#define LATCH_MODEL_GC_RENDER_H
#include <zephyr/kernel.h>

struct gc_bo {
	void *cpu;
	uint32_t gpu;
	uint64_t phys;
	size_t size;
};
int gc_render_init(const struct device *dev);
int gc_render_alloc(size_t size, size_t align, struct gc_bo *out);
#endif
