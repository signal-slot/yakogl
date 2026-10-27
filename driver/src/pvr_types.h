/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_types.h -- OS-independent driver types shared with YakoGL.
 * <stdint.h> only, so host / bare-metal builds of the GL library can include
 * the render-service header without any Zephyr or Linux dependency.
 */
#ifndef PVR_TYPES_H
#define PVR_TYPES_H

#include <stdint.h>

/* A render-pool buffer object mapped into the GPU MMU. */
struct pvr_gpu_bo {
	uint64_t gpu;      /* GPU device virtual address */
	uint8_t *cpu;      /* CPU pointer into the carveout */
	uint32_t heap_off; /* offset within its heap (== exec-base-relative addr) */
};

#endif /* PVR_TYPES_H */
