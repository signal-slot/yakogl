/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gles_osal_baremetal.h - what a bare-metal board gives the bare-metal OSAL
 * (src/osal/baremetal.c) before its init runs.
 *
 * The OSAL has no address of its own: no default register base, no default
 * carveout, no weak hook that does nothing. A board fills a struct
 * osal_bm_board and calls osal_bm_board_config() once, before the first GL
 * or EGL call (that is what runs the OSAL's init). Without it, or with a
 * required field left 0 / NULL, init returns -EINVAL, logs one line that
 * starts "yakogl baremetal: init refused:" and names the field, and touches
 * no address: an integration mistake stops there instead of reading or
 * writing whatever physical address a default named.
 */
#ifndef GLES_OSAL_BAREMETAL_H
#define GLES_OSAL_BAREMETAL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct osal_bm_board {
	/* CPU address of the GPU register block. Required. */
	uintptr_t reg_base;

	/* The GPU's DDR carveout: its physical base (4 KiB aligned; what the
	 * GPU MMU maps), the CPU address the OSAL reads and writes it at (the
	 * physical base itself on an identity map; a board with a CPU MMU
	 * gives its mapping) and its size. All three required. */
	uint64_t carveout_phys;
	void    *carveout_cpu;
	size_t   carveout_size;

	/* Where gpu_alloc starts inside the carveout: the bytes below it are
	 * the board's (firmware heap, page tables). May be 0; must be below
	 * carveout_size. */
	size_t render_pool_off;

	/* Power on, reset, load and boot the firmware. Returns 0, or < 0 and
	 * init returns that. Required. */
	int (*boot)(uintptr_t regs);

	/* Program one 4 KiB-granular GPU-MMU mapping (the port of the driver's
	 * pvr_mmu_map()). Required. */
	void (*mmu_map)(uint64_t va, uint64_t phys, size_t size, uint64_t l0flags);

	/* Data-cache clean (CPU writes -> memory) and invalidate (memory ->
	 * CPU) of a range; on an uncached carveout they may do nothing, but the
	 * board says so by giving them. Both required. */
	void (*cache_clean)(void *p, size_t n);
	void (*cache_invalidate)(void *p, size_t n);
};

/* Registers the board. The OSAL keeps a copy, so `board` need not outlive
 * the call. Returns 0; -EINVAL (-22) for a NULL `board`; -EBUSY (-16)
 * while the OSAL is initialised (the register block and the carveout cannot
 * move under a running GPU). The fields are checked by init, not here. */
int osal_bm_board_config(const struct osal_bm_board *board);

#ifdef __cplusplus
}
#endif

#endif /* GLES_OSAL_BAREMETAL_H */
